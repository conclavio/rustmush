# 21. The graphics engine — one frame, three encoders

**Draft 0.19 — 2026-10-22, G.3b: the web render.** The cockpit picture is drawn in the browser
by the engine's own renderer built to `wasm32-unknown-unknown` (`mush-pixel-wasm`, plain exports,
no wasm-bindgen, 348 KB), from the keyframes the connection already holds; the gateway draws
nothing for a web viewer, and its socket picture is the fallback. Every frame is the same bytes on
every target (the trigonometry through `libm`), pinned by a golden the browser's module matches
hash for hash. The CSP gains `'wasm-unsafe-eval'` and `.wasm` is served as `application/wasm`;
the bundle's builder needs the wasm32 target, the box nothing (cut notes, "G.3b.1"–"G.3b.3").

**Draft 0.18 — 2026-10-22, G.11: the asset pack over the player's own connection.** The pack is
the repository's `assets/` (the manifest and, first, a placeholder sound a notice kind), compiled
into the binary and checked whole against w11term's manifest rules at first use; the SSH server
accepts one more session channel on a connection in game, for the `sftp` subsystem only, and
serves the pack read-only from memory (SFTP v3, 64 KiB reads, 1 MiB/s, 32 handles); the TUI tells
a w11term the pack's version (OSC 440 `pack`) when its `XTVERSION` names it and every ten minutes
after. The web Hello's `asset_pack` is left to the web's first use of the pack (cut notes,
"G.11").

**Draft 0.17 — 2026-10-22, G.10: bells and sounds.** The cockpit's notice kinds are the sound
names; a terminal whose `XTVERSION` reply names w11term is sent w11term's four-field OSC 440 for
every kind, any other the BEL for the few a pilot must hear; one a kind (one bell) every two
seconds, the overheat alarm again on that cadence while the frame says so; `@sounds on|off`.
`XTVERSION` joined the probe and the key parser swallows DCS as it does APC (D-21-24 met). Kitty
gets BEL, not OSC 99: an OSC 99 is a desktop notification, one a sound (cut notes, "G.10").

**Draft 0.16 — 2026-10-22, G.8: the replay as a picture.** `mush_pixel::replay` draws the
`arena.replay` document as a scene over its ground — the whole field, both sides in their team's
colour with headings and trails, the shots as lines for a moment, the wrecks, the focus ringed —
over a timeline of the whole match; `^G` on a replay opens it in the TUI, where space plays it (at
1x, 4x, 16x), `,`/`.` step a keyframe and `[`/`]` ten seconds, and the replay's own panes are the
text under it. Between keyframes it is the renderer's drawing and says so, the web's rule (cut
notes, "G.8").

**Draft 0.15 — 2026-10-22, G.5: the scene model and the orbital view.** `mush_pixel::scene` is
§3.3's model and its one rasteriser (the minimap re-built on it, pixel-equal); `mush_pixel::chart`
draws `sky.chart` as a scene on the chart's own log scale, with the passages and light-lag rings
about the viewer; `^G` on the TUI's chart screen opens it (the picture is the screen's); the web's
chart tab keeps its own SVG and gains the same rings (the owner's four defaults; cut notes, "G.5a",
"G.5b", "G.5c").

**Draft 0.14 — 2026-10-22, G.3c: the web frame's client half.** The cockpit tab draws the
gateway's picture (G.3a) on a canvas above its map, asked for on the viewer's word and only while
the tab is visible; its arrow keys turn the tank with `HEADING` orders as the TUI's do, its `,`/`.`
swing the look (cut notes, "G.3c").

**Draft 0.13 — 2026-10-22, G.1b: the step-down, `[pixel]`, `@pixel/stats`.** §3.4's step-down is
built on what russh was found to do (it stops taking a channel's data while the client's window is
full, so the gateway's write waits — the signal); the clock and the thresholds are `[pixel]` in
`mushd.toml`; the `u` key and the engineering numbers sit behind `[pixel] diagnostics`, a box-wide
switch (the owner's ruling); `@pixel/stats` tells a player their own numbers; the box counts views,
pictures, bytes, slow skips and step-downs in `/metrics` (cut notes, "G.1b").

**Draft 0.12 — 2026-10-22, the text cockpit (D-21-10 revised, D-21-26).** The owner: the player
must read and send chat while navigating, so the picture takes the **top two-thirds** of the
terminal, drawn at that box's shape (640 wide, the renderers' vertical scale following the width so
a wide frame is the same world cropped, not squashed), with only a **flight HUD** in its pixels —
heading tape, reticle boxing the lock, speed bar; the armour doll, weapons and heat, contacts,
notices and the comms move to **terminal text** in the bottom third, and **Enter opens a chat line**
that owns the keys until Enter sends it or Esc goes back to flying (cut notes, "Text cockpit").

**Draft 0.11 — 2026-10-22, the picture measures sight as the sim does.** Both producers now use
the sim's line-of-sight numbers — a 1.5 m eye (a unit's sensor height), 3 m walls, 6 m light and 8 m
heavy woods — mirrored from `mush-atlas` and pinned by a contract test; the raycaster walks past a
block lower than one behind it, units are hidden pixel by pixel, and **a contact the sim reports is
never invisible**: covered by the picture's own ground or walls, or too far to draw, it is
bracketed where it stands (cut notes, "Sight alignment").

**Draft 0.10 — 2026-10-22, G.7a: the heightfield producer.** `mush_pixel::heightfield`, a voxel-space
renderer over a `Terrain` (the arena's cell grid plus an elevation a cell), tested on the proving
ground's shipped relief and a synthetic crater; 2.45 ms a frame at 640x480 on the owner's box. The
proving ground already has relief the sim uses (doc 16 §11, cut X.6); only `arena.map` leaves it out,
so G.7b gives the feed its elevations and, the owner's word, iterates the arena's relief, which "was
just an initial placeholder" — both in `tacsim`, after the mining train (cut notes §G.7a). D-21-20's
Ceres site stays the owner's first-map choice for the surface vehicle's windscreen; the arena is now
the nearer first map for the cockpit.

**Draft 0.9 — 2026-10-22, G.3a: the web frame's gateway half.** A `picture` client command turns
on PNG frames as binary WebSocket messages, drawn per connection in the gateway (`mush-net-web`)
from the cockpit and map keyframes that connection's own stream carries; the web client's canvas
(G.3c; G.3b stays the WebAssembly render) waits for the mining session's SM.10, which holds the web client's
files (cut notes §G.3a).

**Draft 0.8 — 2026-10-22, G.4: the camera on a leash.** The turn keys steer the tank with
`HEADING` orders; the picture predicts the tank's heading between pulses from the order, the last
frame and a turn rate measured from frames; `,` and `.` are a look that orders nothing; on a
terminal that answers the kitty keyboard protocol's query the view pushes it while open and a
turn is held, elsewhere toggled (the owner's three defaults, cut notes §G.4).

**Draft 0.7 — 2026-10-22, G.2b: sixel fills its box.** The owner after the smoke: the native-size
picture in a corner "won't work". A sixel picture is now shown as large as its cell box holds at the
frame's aspect, up to three times the frame: rendered at 640x480 as before and scaled
nearest-neighbour by the encoder, which is what a kitty terminal does with the same frame. The cost
grows with the rows shown, not the area (cut notes §G.2b). Smoked live on dev2: 1717x1288 at
21 fps in a high-DPI w11term, 1013x760 in Windows Terminal over `ssh.exe`, each filling its box.

**Draft 0.6.1 — 2026-10-22, G.2 smoked live on dev2.** Kitty in w11term, sixel in w11term (21 fps)
and sixel in Windows Terminal over `ssh.exe`, all over the real path; a sixel frame is drawn at its
own pixel size, small on a high-DPI terminal that draws sixel at real pixels (cut notes §G.2).

**Draft 0.6 — 2026-10-22, G.2 landed; sixel at 20 Hz.** The sixel encoder, the cell-size probe,
the encoder choice and a per-connection `@pixel/encoder`. Measured on the cockpit fixture, a sixel
frame is 20.6 KB raw against kitty's 9.4 KB but 6.6 KB each after zlib, so the owner set sixel's
rate to the picture's own **20 Hz** (D-21-3 and D-21-16 amended: the 10 Hz default assumed a frame
twice kitty's). Notes in `21-cut-notes.md` §G.2.

**Draft 0.5 — 2026-10-22, G.1 split.** The lead split G.1 in two (§10): **G.1a landed** — the
picture seam, the two-pass raycaster (render plus instruments 4.0 → 0.95 ms a frame at 640x480 on the
owner's box, the same pixels), the probe's DA1 attributes as a graphics record, the help text G.0
owed — and G.1b (the step-down, `[pixel]`, `@pixel/stats`, the `u` key's gate) waits on the mining
session's config change. Notes in `21-cut-notes.md` §G.1a.

**Draft 0.4 — 2026-10-22, after the review of w11term 331a550.** G.10 and G.11 now state the
wire forms w11term implements (`docs/game-client-protocol.md` there): the four-field OSC 440 with
no loop, SFTP v3 with the `read_len` floor, the manifest rules beyond the doc, the DCS parser
change the probe needs first (§10, D-21-24).

**Draft 0.3 — 2026-10-22.** Two cuts added from the owner's work on w11term the same night: G.10
bells and sounds (D-21-24) and G.11 assets over the player's own connection (D-21-25); §10, §11, §14.

**Draft 0.2 — 2026-10-21, the owner's acceptance.** Every Q-21 default was accepted by the owner
the same evening ("all question defaults accepted"), so each stands as a decision; Q-21-8 was
answered differently and better (§12). Draft 0.1 was written the evening the pixel cockpit went live. The owner's ask,
verbatim: *"i think we've proven that not only is this possible, it's pretty fucking cool .. let's go
ahead and write up a design proposal for the graphics engine with its 3 encoders (kitty, sixel, web)
and the variety of use cases that we identified and you fit into three buckets"*. The proof it rests
on is in the tree and on dev2: `crates/mush-pixel` (a software raycaster over the arena grid, a
palette PNG encoder, the kitty graphics framing, the instruments drawn into the frame) and the SSH
TUI's `^G` view (`crates/mush-net-ssh/src/tui/pixel.rs`), measured live from WezTerm's own SSH
client against the VPS, with every number in `crates/mush-pixel/NOTES.md`. Two rulings of the day
stand under it: **640x480 at 20 frames a second is the target**, and **the instruments are drawn
into the frame** (minimap, heat, weapons, the armour doll, the status strip), because "we'll
appreciate the extra pixels".

What this document does is turn the spike into an engine with a place in the architecture: a
per-connection picture, produced from the same per-observer feeds every other renderer reads,
encoded three ways — the kitty graphics protocol, sixel, and the web — and put to work across the
fifteen-odd uses the owner listed, which fall into three buckets with three different costs. It
decides the shape (D-21-n, inline and listed in §11), asks the owner what only the owner can
answer (Q-21-n, §12, each with the default the build takes), and cuts the work (G.n, §10). No
deploy without the owner.

---

## 1. What stands, what this document decides

**Stands.** The one-protocol stream (doc 02): a connection receives `OutputEvent`s, among them
`Panel` keyframes and deltas on named feeds (`arena.cockpit`, `arena.map`, `sky.chart`, the unit
and vitals feeds), each already projected for *that observer* through the epistemic seam, and three
renderers draw the same stream — line mode (`mush-ansi`), the SSH TUI (ratatui), the web client.
The cockpit feed at version 6 carries the pilot's `x_cm`, `y_cm`, `heading_mdeg`, every contact's
bearing, range, class and cell, the hull's sections and facets, the weapons, the heat. The arena map
feed carries the cell grid, its legend and the named spots. The forked split deploy (gateway +
server over IPC) keeps the connection in the gateway half across a server restart.

**Stands from the spike (2026-10-21).** `mush_pixel::Frame` (packed RGB), `raycast` (DDA over the
cell grid, floor cast, fog, billboard sprites), `png` (palette when ≤256 colours), `kitty` (probe,
chunked inline frame, delete), `overlay` (the instruments, a 5x7 font, a closed palette), `cockpit`
(the feed-to-inputs adapter). In the TUI: the active capability probe at `@tui` entry, APC and DA1
decoded as marker keys, the frame clock as a per-connection deadline in the output pump, pictures
only on the clock, an idle tick writing nothing, a frame sent only when its hash or its cell box
changed, the image freed on close. Measured on the 4-vCPU VPS for one viewer turning: 640x480 at
20 Hz costs 21 % of a vCPU and 100 KB/s; uncapped the box renders 189 frames a second at 320x220
and 58 at 640x480; the server half never exceeds 2 %.

**Decides.** That the picture is a **fourth renderer of the feeds**, not a new feed (§3, D-21-1);
that one `Frame` feeds **three encoders** chosen by what the client proves it can show (§4,
D-21-2..5); that the uses fall into **three buckets** with their own producers and cadences (§2,
D-21-6..8); the budgets and the clock (§5); the client matrix as measured, including that Windows
reaches kitty only through a terminal's own SSH client (§6); the input and the camera's relation to
the sim (§7); the cuts (§10).

**Refuses.** A video codec, a GPU, 3D meshes, alpha compositing over text, tmux in the first
version, any picture the observer's feeds do not already entitle them to (§13).

## 2. The three buckets — D-21-6

Every use the owner named on 2026-10-21 (the cockpit and its kin; the orbital view; naval combat
plots; corridor views in room descriptions; the trading interface; then damage control, vehicle
fitting, surveying, boarding and deck plans, industrial layout, docking and salvage, remote
equipment, the strategic board, cargo handling, debrief and replay, investigation boards) is one of
three kinds of picture, and the kind sets the producer, the cadence and the cost.

**Bucket A — first-person raster.** A camera in the world, a new picture whenever it or the world
moves, at the frame clock's 20 Hz. The producer is a renderer over a spatial model: today the
raycaster over the arena's cell grid; later a heightfield renderer where the ground has relief
(§8). Its instruments are drawn into the same frame. Cost: the render dominates (6–8 ms a frame at
640x480 today), so this is the bucket that consumes the budget in §5.

| Use | Camera | Spatial model | Notes |
|---|---|---|---|
| The arena cockpit (built) | the seat | `arena.map` cells + `arena.cockpit` | the proof |
| Surface mining — the TV's windscreen (doc 20) | the rig | the site's atlas region + geology chart | the same raycaster over the site map; the `+tv/map` grid is already its minimap |
| Ground fighting on a body's surface (doc 17 R-B's end state) | a unit | the IC region map | bucket A once the region carries a map; the arena cockpit generalised |
| Remote equipment: drone, rover, turret, station camera | the device | wherever it stands | a seat is a seat: the control channel (F3) decides whose picture it is |
| A corridor view in a room description (Ceres) | the room | a corridor model the atlas does not carry today | **deferred**: needs authored geometry per room (Q-21-6); a stock "corridor" render from the neighbourhood's graph is the cheap first form |

**Bucket B — diagrams and maps.** A retained 2D scene — shapes, lines, text, sprites, an image or
two — drawn from a feed and redrawn when the feed changes, never faster than a few frames a second.
The producer is a *scene builder* per use that turns feed JSON into the scene model of §3.3, and one
rasteriser draws every scene. Cost: a frame on change, a few milliseconds, bytes like a bucket-A
frame; at ≤5 Hz a viewer costs a twentieth of a cockpit.

| Use | Feed it draws | First form |
|---|---|---|
| The orbital view — the system, the lanes, your dot from Phobos to Ceres (doc 19) | `sky.chart` (pulled, never pushed: asked for on opening and on `r`; the TUI's screen `c` draws its flat map and the web's chart tab an SVG — *as built G.5: and the TUI's `^G` the picture*) | the chart as a picture: bodies, orbits, the plan's legs, the ship's dot, light-lag rings |
| Naval ship-to-ship plots: ships, torpedo trails | a plot feed the naval ruleset does not exist to push yet | reserved: the scene model takes polylines with age, which is what a trail is |
| Damage control: compartments, pressure, fires, breaches, power, repair crews | the hull's `sections` today; a compartment feed for ships later | the armour doll stands alone and grows a legend; compartments when ships have them |
| Vehicle fitting and maintenance: the paper doll, hardpoints, arcs, ammunition | the unit sheet (`unit` feed) + the sections | the doll with hardpoints; the arcs drawn as sectors |
| Surveying and prospecting: coverage, deposits, cross-sections (doc 20) | `tacsim.chart` (SM.4a) and the site map | the TV's chart as a picture: coverage shading, pocket marks with uncertainty rings |
| Boarding, infiltration, derelicts: deck plans, doors, airlocks, squads | the atlas region (rooms, exits) + a party feed | the neighbourhood graph drawn as a plan; discovery shades what the party has seen |
| Industrial operations and outposts: the flow from mines to refineries | the industry pools (E-series) | a node-and-edge diagram; stalled nodes red |
| Docking, EVA, salvage: relative motion, alignment, tethers | the journey/route primitives (doc 06) at a close scale | the same plot at a different scale and origin |
| The strategic board: claims, patrols, supply routes, last-seen contacts with age | the faction fronts and clocks (M25) + claims (doc 20 §9) | the map with age-faded marks; annotations need a store (G.9) |
| Cargo handling: the hold diagram | the hold's stow (doc 18 §4.5) | boxes in a hold; blocked access drawn when arrangement matters |
| Debrief and replay with a scrubbable timeline | the tactical replay document (16b I.2) + the scene log (M22/M23) | the replay's grid keyframes as a picture with a time slider (keys, then a mouse) |
| Investigation boards: people, ships, places, transactions, evidence | the canon ledger and recaps (M23), player annotations | a graph diagram with pins; the annotations store is the work (G.9) |

**Bucket C — forms and documents.** Listings, quotes, orders, the fitting menus, the sheets. These
are text panels and stay text panels: the existing `Panel` feeds and the three text renderers draw
them already, and a picture adds nothing a column does not. Where a form wants a picture (the
fitting screen's doll, a listing's thumbnail of a hull) it embeds a bucket-B scene as one image
beside the text. Cost: nothing new.

**D-21-6.** A use is classified by what changes its picture: the camera (A), the feed (B), the
player's typing (C). A and B share the `Frame`, the encoders, the clock and the caps; C shares
nothing but the embedding. The buckets are not a priority order — the cuts in §10 take one of each
early so the engine's three producers are all exercised before any is deep.

## 3. The engine

### 3.1 A fourth renderer, not a feed — D-21-1

The picture is rendered **in the gateway half, per connection, from the feeds that connection
already holds** (`PanelStore`), exactly as the TUI's panes are. Nothing pixel-shaped crosses the IPC
boundary, the server half stays pixel-free, and the epistemic seam is honoured for free: a viewer's
picture can only show what their feeds carry (the range targets on dev2 were not on the minimap
until the sim listed them as contacts — that is the rule working, not a bug). A picture survives a
server restart with its connection, as the TUI does.

The one feed change this needs is none: `arena.cockpit` v6 and `arena.map` v1 carry the cockpit;
`sky.chart` carries the orbital view; `tacsim.chart` the survey. A bucket-B use whose feed does not
exist yet (a naval plot, a compartment map) is a feed to design, not a picture to design.

### 3.2 The frame and the producers — D-21-7

`mush_pixel::Frame` is the one picture: packed RGB, `width` by `height`, with a hash and a colour
count. **Three producers** write into it:

- **`raycast`** (built): the first-person render over a `Map` of `Ground` cells, a `Camera` and
  billboard `Sprite`s.
- **`heightfield`** (G.7): the first-person render over a height-and-colour map, Comanche-style
  voxel-space, for ground with relief; chosen per spatial model, not per use.
- **`scene`** (G.5): the bucket-B rasteriser over a retained scene (§3.3).

and **`overlay`** draws the instruments over any of them. A producer is a plain function over plain
inputs — no feed JSON inside `mush-pixel` except in the `cockpit` adapter — so it is testable from a
fixture and compiles anywhere, including to WebAssembly (§4.3).

**Colour discipline.** Every producer draws from a closed set of colours so a frame never exceeds
256 and the PNG is always indexed (the raycaster's 16 bases times 12 fog steps plus 16 sky; the
overlay's two dozen; a scene's palette declared with the scene). The encoder's truecolor fallback
stays as a safety net and is logged when it fires. No dithering: it quadruples the bytes for nothing
at these palettes.

### 3.3 The scene model — D-21-8

Bucket B's retained scene, `mush_pixel::scene::Scene`: a viewport (world rectangle to frame
rectangle, with a transform for the orbital view's scale), layers in order, and items — `Line`,
`Polyline` (with per-vertex age, for trails), `Rect`, `Circle`, `Arc`/`Sector` (firing arcs, light
rings), `Text` (the 5x7 font, scaled), `Sprite` (a small bitmap by name from a sprite sheet the
scene declares), `Image` (an embedded `Frame`, for a doll inside a plan) — each with a colour from
the scene's palette and an optional id. A scene builder per use produces it from a feed; the
rasteriser draws it; the overlay may add a status strip. A scene is also what the minimap is: the
cockpit's minimap becomes `scene` the day the rasteriser lands, drawn into the bucket-A frame
through `Image`.

### 3.4 The clock, the hash, the cap — D-21-9

Per connection, in the output pump: a **deadline clock** (`sleep_until`, resynced when behind) at
the view's interval, 50 ms for bucket A; pictures go out **only on the clock** (an event repaint
draws text, never a picture — the arena's per-pulse repaints had doubled the rate); a tick whose
frame hashes the same as the last sent, in the same cell box, **writes nothing**; a picture is sent
inside synchronized output with the text frame refreshed under it. Bucket B's clock is the same
mechanism at 200 ms, which with the hash is "on change".

**Back-pressure.** The write to the channel is bounded by the pump's existing 5 s timeout, which is
the wrong unit for a picture. G.1 adds the step-down: when a write takes longer than one interval,
the next frame is skipped; when three in a row do, the view steps to the low frame (320x220) and to
10 Hz, and steps back up after ten clean seconds. A client that stops reading costs the gateway one
frame and then nothing, which is what the first live run taught (a frozen client's SSH window filled
with the HUD's twenty writes a second).

### 3.5 Placement — D-21-10

The frame is placed as **one image** under one fixed id and placement, so there is one image to
replace a tick, one id to free, and no z-order to reason about. A bucket-C form that embeds a
bucket-B scene places it the same way, as one image beside the text.

**Revised 2026-10-22 (the text cockpit).** The image takes the **top two-thirds** of the terminal
under the status line, and the frame is drawn at that box's shape — 640 (or 320) wide, its height
from the box in the terminal's cell pixels, between 4:1 and 4:3 — so a kitty terminal scales it over
the whole box and a sixel one is encoded up to fill it. Only the **flight HUD** stays in the pixels
(heading tape, reticle and lock box, speed bar), because those are read off the view; the
instruments that are numbers and states — the armour doll, weapons and heat, contacts, notices —
are **terminal text** under the picture, beside the comms, because the player has to read and send
chat while flying (the owner, superseding "instruments inside the frame" of 2026-10-21). The
instruments-in-the-frame overlay stays in `mush-pixel` as `Overlay::Instruments` for a client with
no text beside the picture.

## 4. The three encoders

One `Frame`; the encoder is chosen once per connection by what the client proved, and never sees
game data.

### 4.1 Kitty — D-21-2 (built)

The kitty graphics protocol, as the spike does it: PNG (`f=100`) sent inline (`t=d`) in 4096-byte
base64 chunks, `a=T` under a fixed `i` and `p`, `q=2`, `C=1`, `c`/`r` for the cell box, the whole
frame between `CSI ?2026h` and `CSI ?2026l`. Detection is **active**: a 1x1 query image under id 31
followed by `CSI c`; an OK before the DA1 reply is support, the DA1 reply first (or 500 ms of
silence) is not; replies arrive as marker keys through the input parser, which swallows every other
APC. Verified in WezTerm (its own SSH client) and, by the protocol, kitty and Ghostty.

### 4.2 Sixel — D-21-3 (G.2)

The same frame as a DEC sixel stream: `DCS q`, the raster attributes, a palette of the frame's
colours (its ≤256 by construction), six-row bands of run-length-encoded sixels, `ST`. Placed by
cursor position at the cell box's corner; there are no ids or z-order, so the whole picture is
redrawn every sent frame and the text frame is drawn around it. Detection is **passive and free**:
the DA1 reply the probe already elicits lists attribute `4` when the terminal does sixel, so the
parser's `Key::DeviceAttributes` grows the attribute set and the view reads it. Verified by hand
with the MechWarrior 2 frame in Windows Terminal Preview 1.26 (and by the protocol: foot, xterm,
mintty, mlterm). Bytes: about twice the PNG after SSH's own zlib on flat frames (the MW2 frame
measured 87 KB against 49 KB), so sixel's default cap is **10 Hz** (Q-21-1), and it follows the
same step-down. *As built (G.2, 2026-10-22): **20 Hz**, the owner's
ruling once the cockpit fixture measured 20.6 KB raw against kitty's 9.4 KB but 6.6 KB each after
zlib (the MW2 frame's ratio is a busy textured frame's, not the flat cockpit's); the frame is sized
to the cell box from the terminal's `CSI 16 t` answer, sent again when text overwrites it, wiped by a
full repaint on close (cut notes §G.2).*

### 4.3 Web — D-21-4 (G.3, G.3b)

Two forms, the first a week's work and the second the right one:

- **G.3, the frame over the websocket.** The gateway already renders the frame for an SSH viewer;
  for a web viewer it sends the same PNG as a binary message on the session's socket, and the
  client draws it into a `<canvas>` in the cockpit tab. Zero client logic beyond `drawImage`; the
  same bytes as kitty; the same clock and hash. This makes the web client the third encoder with the
  least code and proves the feed path end to end.
- **G.3b, the picture rendered in the browser.** `mush-pixel` has no dependency but `flate2` and
  `serde_json`, so it compiles to WebAssembly as it stands. The web client then runs the *same*
  producers over the *same* feeds it already receives (`arena.cockpit`, `arena.map`, `sky.chart`)
  and draws into the canvas at whatever rate the browser likes, with the gateway's CPU and the
  link's bytes at zero. One crate, three homes; the fixture test that pins the TUI's and the web's
  text renders equal (16b Q.5) gets a sibling that pins the picture equal.

**D-21-4.** The web client's first encoder is the gateway's frame (G.3); the WebAssembly render
(G.3b) replaces it for bucket A and B alike once it draws the fixture pixel-for-pixel, and the frame
over the socket stays as the fallback for a browser without WebAssembly.

### 4.4 Choosing — D-21-5

Per connection, once, in this order: kitty if the probe said OK; else sixel if DA1 lists 4; else no
picture, and the TUI's text panes are the cockpit as before. The web client is always its own case.
A terminal that lies (tmux without passthrough swallows the probe and then DA1 says what the outer
terminal does) gets what it asked for; tmux is unsupported in the first version and says so in
`help cockpit` (§13). The owner may force an encoder per account later (Q-21-2).

## 5. The budget

Measured 2026-10-21 on the 4-vCPU VPS, one viewer turning, the box sampled for 14 s each run:

| frame | clock | fps on the wire | render | encode | wire a frame | SSH bytes | gateway CPU |
|---|---|---|---|---|---|---|---|
| 320x220 | 20 Hz | 20–21 | 0.5–2.6 ms | 0.3–1.4 ms | 1.5–3.3 KB | 24–44 KB/s | 4–8 % |
| 640x480 | 20 Hz | 21 | 6.2 ms | 1.0 ms | 6.6 KB | 100 KB/s | 21 % |
| 640x480 + instruments | 20 Hz | 21 | 7.6–8.6 ms | 1.1 ms | 8.9 KB | ~110 KB/s | ~25 % |
| 320x220 | uncapped | 189 | 2.3 ms | 0.4 ms | 2.7 KB | 368 KB/s | 51 % |
| 640x480 | uncapped | 58 | 10.5 ms | 1.4 ms | 7.8 KB | 284 KB/s | 75 % |

The server half stayed under 2 % throughout; the picture costs the world nothing. A connection's
pump is one task, so a viewer takes at most one core, and the render is the ceiling.

**The target's budget (D-21-11):** 640x480 at 20 Hz is **a quarter of a vCPU and 110 KB/s a
viewer**, so the present box carries about sixteen bucket-A viewers with everything else running,
sixty-odd at the low step, and bucket-B viewers at ≤5 Hz for a twentieth of that. The planning
number for the launch box is twenty bucket-A viewers a vCPU after the one optimisation the spike
left on the table: the floor cast is per pixel with a map lookup each and is where 640x480 spends
its time; a per-row ground lookup halves it (G.1's acceptance). The VPS's 32 TB a month is not the
constraint at these rates (a viewer at the target is 400 MB an hour); the port and the per-player
downlink are, which is what the step-down is for.

## 6. The clients, as measured — D-21-12

| Client | kitty | sixel | How the bytes reach it | Finding |
|---|---|---|---|---|
| WezTerm (Windows, 20240203) | yes, **through its own SSH client only** | yes | `wezterm ssh` or an SSH domain | ConPTY strips the kitty APC for anything run as a console process, `ssh.exe` included; the spike's frames drew through `wezterm ssh` and nothing through `cmd`; its OpenGL front end hung the owner's PC once on the first `^G`, the software front end never did (§13) |
| Windows Terminal (1.24 stable, 1.26 preview) | **no** (1.25 added the kitty *keyboard* protocol; graphics is microsoft/terminal#8389, open, a design spec from July 2026) | yes, through ConPTY | `ssh.exe` | the MW2 sixel frame drew first time; **sixel is the Windows path**, not the fallback |
| kitty, Ghostty 1.2+ | yes | kitty: yes | their own SSH | by the protocol; pixel's row-diff patching exists only here and is not needed at these frame sizes |
| foot, xterm, mintty, mlterm | no | yes | ssh | by the protocol |
| tmux | passthrough + placeholders | passthrough | — | unsupported in v1 |
| **w11term** (the owner's own, `codexprojs/w11term`) | yes (wezterm-term's parser) | yes | its own SSH client, no ConPTY | a Windows-native terminal the owner built around the `wezterm-term` library with a `windows-reactor` + Direct2D front end; the shell is the SSH server, so nothing passes through ConPTY; faster than WezTerm and "feels like a windows application"; the owner means to publish it beside the game as the Windows option (Q-21-8) |
| Web client | — | — | websocket (G.3), then WebAssembly (G.3b) | the one client with zero wire cost at G.3b |

**D-21-12.** The engine supports both terminal encoders from the first cut after this document
because the owner's own desktop needs sixel for Windows Terminal and kitty for WezTerm, and a
Windows player typing `ssh` in PowerShell will only ever get sixel.

## 7. Input and the sim — D-21-13, D-21-14

**Toggle-turn stands as the first input** (press to start turning, press again to stop, space to
hold, `c` to recentre): it needs no key-release events and it is how the spike was driven. Hold-to-
turn comes with the kitty keyboard protocol (`CSI … u`, release events; WezTerm, kitty, Ghostty and
Windows Terminal 1.25+ speak it) as G.4's second half, negotiated the way the graphics are, and the
key parser already has the CSI path for it. Mouse-look is later and needs SGR pixel mode.

**The camera on a leash (D-21-14).** The spike's camera turns locally and the sim is deaf; the
picture therefore shows a heading the tank does not have. G.4 closes that the only honest way: a
turn is a **helm order** through the control channel (F3's `helm` channel, the same authority the
`HEADING` verb uses), the sim resolves it at its tempo, and the view **interpolates** the pose
between pulses while showing the local camera offset as what it is — a look, not a turn — until the
pulse lands and the two reconcile. The camera never claims state the sim has not granted; the
contacts a frame lists are the contacts the sim detected; a wreck is drawn when the frame says
`reads_dead`. This is the epistemic rule of doc 02 applied to pixels, and it is why the renderer
lives on the feeds.

## 8. The heightfield (G.7) — D-21-15

The raycaster draws walls and floor: right for an arena of blocks and a corridor, wrong for a
crater rim. The second bucket-A producer is a voxel-space heightfield renderer (Comanche's: a
height map and a colour map, columns cast back to front, about 150 lines) chosen whenever the
spatial model carries height. Today no map does: the arena grid is flat and doc 20's geology is
provinces and pockets, not relief. **D-21-15:** the heightfield is built when a map carries it,
and the first map to carry it is the one the owner picks (Q-21-5): the proving ground's `atlas`
region with authored contours, or a Ceres site from doc 20 with the geology key seeding a height
field the same way it seeds pockets (cheap, deterministic, and a crater looks like a crater).

## 9. What the spike leaves to the engine

- The `u` uncapped switch stays as a staff diagnostic behind a privilege, never a player key.
- The HUD line the TUI prints above the picture (render, encode, bytes, fps) becomes a `@pixel/stats`
  verb and a line in `healthz`; the frame's own status strip is the player's.
- `320x220` stays as the step-down frame (D-21-9) and as the `r` key for a player on a slow link.
- The probe's 500 ms is the cost of `@tui` on a terminal without graphics; it runs once a
  connection and in parallel with the TUI's own hydration queries, so it is invisible.
- `FRAME_INTERVAL` and the step-down thresholds move to `[pixel]` in `mushd.toml`.
- The example `cargo run -p mush-pixel --example fixture` stays the way to look at a frame without
  a terminal, and grows a `--scene` flag for bucket B.

## 10. Cuts

Each cut is one commit on `main` with the Fable trailer, pushed when green, with a `## G.n` heading
in **`docs/design/21-cut-notes.md`**. Sizes as doc 18 §5: S a day, M a few days, L a week or more.
No deploy without the owner; dev2 is the box these cuts smoke on.

| Cut | What | Size | Depends on | Acceptance |
|---|---|---|---|---|
| **G.0** | This document; the README row; the cut notes started; `help cockpit` names `^G`, the keys, the terminals that work and tmux that does not | S | — | the README row; help text |
| **G.1 the engine from the spike** | `mush_pixel` promoted: the producer seam (`Picture` = producer + overlay + clock interval), the step-down on back-pressure (§3.4), `[pixel]` config, `@pixel/stats`, the floor-cast per-row lookup, the `u` key behind a privilege, the probe's attribute set. **Split 2026-10-22:** **G.1a (landed)** the seam, the two-pass render, the attribute set and the help; **G.1b (landed)** the step-down, `[pixel]`, `@pixel/stats`, the `u` gate behind `[pixel] diagnostics` and the box counters | M | — | 640x480 render ≤4 ms on the VPS (G.1a: 0.95 ms on the owner's box, the VPS figure owed to G.1b's dev2 smoke); a client that stops reading costs one frame; the step-down test; the contract gates |
| **G.2 sixel** | the sixel encoder from the same frame; DA1 attribute 4 detection; the 10 Hz default; the encoder choice (§4.4); smoked in Windows Terminal over `ssh.exe`. **Landed 2026-10-22** at 20 Hz (D-21-3 amended), with the `CSI 16 t` cell size, the frame sized to the box, the resend after text and the wipe on close, and `@pixel/encoder` per connection | M | G.1 | the MW2 fixture frame and the cockpit frame byte-compared against a reference encoder; the live smoke in Windows Terminal (as built: a decoder round trip in place of a reference encoder; the fixture shown in Windows Terminal through ConPTY; the live SSH smoke owed) |
| **G.3 the web frame** | the gateway's PNG over the websocket as a binary message; the cockpit tab's canvas; the same clock and hash. **Split 2026-10-22:** **G.3a (landed)** the gateway half; **G.3c (landed)** the cockpit tab's canvas — the `picture` command, the per-connection feed cache, the frame clock, the binary PNG; **G.3c** the cockpit tab's canvas in the web client, after SM.10 | S | G.1 | the web cockpit shows the picture the TUI shows, pinned by a fixture test |
| **G.3b the web render** | **Landed 2026-10-22** in three (G.3b.1 libm maths + the golden, G.3b.2 the module + the Node parity test + CI, G.3b.3 the browser draws it, the CSP and the type); the cockpit picture only (the chart and the replay keep the web's SVG and canvas; the heightfield waits on G.7b); `mush-pixel` to WebAssembly; the client renders from its own feeds; the socket frame as the fallback | M | G.3 | the fixture renders pixel-equal in the browser and the TUI; zero gateway CPU for a web viewer |
| **G.4 the leash and the keys** | the turn as a helm order through the control channel; interpolation between pulses; the kitty keyboard protocol for hold-to-turn. **Landed 2026-10-22**: `HEADING` orders a pulse and a half ahead, the heading predicted from the order and a measured turn rate, `,`/`.` a look, the protocol pushed only while the view is open | M | G.1 | the sim's heading and the camera's reconcile within one pulse; a release event stops a turn |
| **G.5 the scene model and the orbital view** | `scene` (§3.3) and its rasteriser; the first builder: `sky.chart` as a picture with the plan's legs, the ship's dot and light-lag rings; the minimap re-based on `scene`. **Landed 2026-10-22** in three: **G.5a** the scene and the pixel-equal minimap, **G.5b** the chart picture on the TUI's chart screen, **G.5c** the rings on the web's SVG (the web keeps its vector chart, the owner's default); the ship's dot is the viewer's port until the feed carries ships under way | L | G.1 | `+chart` draws the picture in every encoder; the minimap pixel-equal before and after |
| **G.6 the doll and the fitting screen** | the armour doll as a standalone bucket-B picture with a legend; the fitting paper doll with hardpoints and arcs embedded in the sheet (bucket C) | M | G.5 | the fixture doll; the fitting screen on the TUI and the web |
| **G.7 the heightfield** | the voxel-space producer; the first map with relief (Q-21-5). **Split 2026-10-22:** **G.7a (landed)** the producer, tested on the proving ground's shipped relief; **G.7b** `arena.map` carries elevations, the cockpit picture draws them, the arena's relief reworked (owner: the arena map is a placeholder), after the mining train | M | G.1, a map with height | a crater rim on the proving ground or a Ceres site at ≤6 ms a frame (G.7a: 2.45 ms on the owner's box over the proving ground; the VPS figure with G.7b's smoke) |
| **G.8 debrief and replay** | the replay document (16b I.2) as a bucket-B picture with a scrubbable timeline. **Landed 2026-10-22** in the TUI (`^G` on a replay): the whole field over a timeline, played at 1x/4x/16x, stepped by keyframe and by ten seconds; the web keeps its own canvas and scrubber (it had both since 16b I.2); a live `@arena/watch` picture is not drawn yet | M | G.5 | `SIM REPLAY` draws the match; `,`/`.` scrub it |
| **G.9 the boards** | the strategic board and the investigation board; the player annotation store they share | L | G.5, an annotation store | two players see each other's pins |
| **G.10 bells and sounds** | **Landed 2026-10-22** (the TUI; kitty gets BEL, the cut notes say why); the cockpit's notice kinds as sound names (`contact-gained`, `contact-lost`, `hit`, `overheat`, `power-online`, …; 1–64 of `[A-Za-z0-9_-]`); BEL for every terminal; **w11term's OSC 440, wire version 1** — `ESC ] 440 ; 1 ; play ; <name> ST` and `ESC ] 440 ; 1 ; pack ; <pack_version> ST`, four fields, no options, no loop (w11term's `docs/game-client-protocol.md`; mintty's older `440;file` form is NOT this and mintty gets BEL) — only for a terminal whose `XTVERSION` reply names w11term; kitty's OSC 99 with its `s=` sound for kitty; the client plays serially at a 40 ms floor with a 16-deep queue and ≤5 s clips, so an alarm is the server re-sending its name on a cadence, debounced server-side at one sound a kind per two seconds; `@sounds on|off`; **`XTVERSION` joins the `@tui` probe, which first needs the TUI key parser to swallow DCS (`ESC P … ESC \`) the way it swallows APC** — today that reply would decode as F1 plus typed text (D-21-24) | S | G.1 | a contact gained rings in w11term and in Windows Terminal's bell; the overheat alarm repeats while the frame says so and stops when it clears; the sound names are the notice kinds; a DCS reply never reaches the input line |
| **G.11 assets over the player's own connection** | **Landed 2026-10-22** (the pack compiled into the binary rather than a deployed directory, so it ships with the gateway that serves it; sounds only, placeholders; the web Hello's `asset_pack` left for the web's first use); the gateway's SSH server accepts the `sftp` subsystem (russh-sftp, **protocol v3**) over a READ-ONLY virtual tree rooted at a shipped `assets/` directory — open/read/close/opendir/readdir/stat/lstat/fstat/realpath only, paths canonicalised, a size cap a file and a bytes-a-second cap a connection; if it advertises `limits@openssh.com` its `read_len` is ≥ 16384 (the client reads 16 KiB and refuses larger-than-limit reads); `assets/manifest.json` as w11term's protocol defines it — schema `version` 1, `pack_version` an OSC-grammar token, `files[{name, path, kind, sha256, size}]`, every `size` > 0, names and the pack version 1–64 of `[A-Za-z0-9_-]`, sound names never `system`/`off`, paths of two or more segments with no `.`/`..`, `.wav` lower-case, no unknown fields (the schema is frozen at 1) — and the Hello's `asset_pack {version, pack_version, manifest}` for the web client; the gateway announces the pack once after login and once after a deploy (an announcement during a running sync is dropped by the client, so re-announce on a slow cadence, never per frame); the first pack stays small (sounds only, well under 10 MiB: sequential 16 KiB reads fetch ~25 MiB in the client's 180 s at 100 ms RTT); w11term syncs what changed into its per-host cache and OSC 440 names resolve there first; the sprite sheet, a font with the HUD's glyphs and the help topics ride the same manifest (D-21-25) | M | G.10, deploy.sh shipping `assets/` | w11term fetches the sound pack on first connect and plays G.10's sounds from it; a write, a rename or a `..` is refused; the manifest version in the Hello changes when the pack does |

## 11. Decisions taken here

- **D-21-1** The picture is a fourth renderer of the per-observer feeds, rendered in the gateway
  half per connection; nothing pixel-shaped crosses IPC; the server stays pixel-free.
- **D-21-2** Kitty: inline PNG under one id and placement, active probe, APC replies as marker keys.
- **D-21-3** Sixel: the same frame as a whole-picture DCS stream, DA1 attribute 4 detection, 10 Hz
  default. *Amended 2026-10-22 (owner, at G.2): 20 Hz, the picture's own clock — measured, a sixel
  frame compresses to a kitty frame's bytes; the frame sized to the cell box from `CSI 16 t`.*
- **D-21-4** Web: the gateway's frame over the websocket first; the WebAssembly render of the same
  crate replaces it; the frame stays as the fallback.
- **D-21-5** Encoder choice per connection: kitty, then sixel, then text; the web is its own case.
- **D-21-6** Three buckets by what changes the picture: the camera (A), the feed (B), the typing (C);
  A and B share everything but the producer; C embeds B.
- **D-21-7** One `Frame`, three producers (raycast, heightfield, scene), one overlay; a closed colour
  set per producer so the PNG stays indexed; no dithering.
- **D-21-8** The scene model: viewport, layers, lines/polylines with age, rects, circles, sectors,
  text, sprites, embedded frames; one rasteriser for every bucket-B use.
- **D-21-9** The deadline clock per connection; pictures only on the clock; idle writes nothing;
  the step-down to 320x220 and 10 Hz on back-pressure.
- **D-21-10** One image a frame, placed into the cell box by aspect. *Revised 2026-10-22:* the
  picture is the top two-thirds, drawn at that box's shape, with the flight HUD alone in its pixels;
  the doll, weapons and heat, contacts, notices and comms are terminal text under it.
- **D-21-11** 640x480 at 20 Hz is the target and the budget is a quarter of a vCPU and 110 KB/s a
  viewer; twenty bucket-A viewers a vCPU after G.1.
- **D-21-12** Both terminal encoders ship early because Windows needs sixel and WezTerm needs kitty.
- **D-21-13** Toggle-turn first; the kitty keyboard protocol for hold keys; mouse later.
- **D-21-14** The camera on a leash: a turn is a helm order, the view interpolates, the picture
  never claims what the sim has not granted.
- **D-21-15** The heightfield producer is built when the first map carries height.
- **D-21-16..22** The Q-21-1..7 defaults, accepted by the owner 2026-10-21: sixel at 10 Hz (amended to 20 Hz at G.2); an
  encoder may be forced per account; the web frame before the WebAssembly render; the orbital view
  is the first scene; a Ceres site is the first map with height; corridor views deferred; no
  one-shot picture in line mode in v1.
- **D-21-23** w11term, the owner's Windows-native terminal on `wezterm-term`, is the reference
  Windows client (Q-21-8).
- **D-21-24** Sounds are names, never audio: the cockpit's notice kinds, sent as BEL everywhere
  and as w11term's four-field OSC 440 (`440;1;play;<name>`) or kitty's OSC 99 where `XTVERSION`
  or the probe says so; the client maps names to files it holds, refuses paths from the server,
  and debounces; an alarm is a repeated name, never a loop (G.10). Amended 2026-10-22 from
  mintty's form to w11term's after the review of w11term 331a550.
- **D-21-26** The text cockpit's chat line (2026-10-22): Enter opens it over the flying view,
  stopping a held turn; while open it owns every key (typing never steers); Enter sends the line as
  typed and returns to flying; Esc returns keeping the draft, which the next Enter brings back; Esc
  while flying closes the view.
- **D-21-25** Client-side assets (sounds, sprites, a font, help) are fetched over the player's
  own SSH connection through a read-only `sftp` subsystem on the gateway, from a manifest with
  content hashes whose version the Hello carries; never over a second port or a second login
  (G.11).

## 12. Questions for the owner — all accepted 2026-10-21

The owner accepted every default below on 2026-10-21 ("all question defaults accepted"); each is
now a decision of this document, D-21-16..22 in the questions' order (§11). Q-21-8's answer
superseded its default (D-21-23).

- **Q-21-1** Sixel's default rate. *Default:* 10 Hz, because the bytes are twice the PNG's and
  Windows Terminal is the usual sixel client on a home link; `r` and the step-down apply.
- **Q-21-2** Forcing an encoder per account (`@pixel/encoder sixel`) for a terminal that lies.
  *Default:* yes, one verb, in G.2.
- **Q-21-3** The order of the web cuts. *Default:* G.3 (the frame over the socket) before G.3b
  (WebAssembly), because G.3 is a day and proves the path; G.3b when the picture is worth a second
  renderer's maintenance.
- **Q-21-4** Which bucket-B use is first. *Default:* the orbital view (G.5), because `sky.chart`
  already exists, the owner named it second, and it exercises every scene item but sprites.
- **Q-21-5** The first map with height. *Default:* a Ceres site, seeded from the geology key, so
  the TV's windscreen (bucket A on Ceres) and the survey chart (bucket B) share one ground.
- **Q-21-6** Corridor views in room descriptions. *Default:* deferred past G.9; a stock render from
  the neighbourhood graph is the only cheap form and it may not be worth the pixels.
- **Q-21-7** Whether a player without graphics should see a one-shot picture in line mode (`+view`
  printing a frame once, for a terminal that can show one but is not in the TUI). *Default:* no in
  v1; the TUI is the picture's home.
- **Q-21-8** The WezTerm freeze. *Default:* the help page recommends `front_end = "Software"` for
  WezTerm on Windows until the owner has investigated; nothing in the engine changes for it.
  **Answered otherwise (owner, 2026-10-21):** the owner got around the freeze by building
  **w11term** (`C:\Users\faeni\codexprojs\w11term`), a Windows-native terminal around the
  `wezterm-term` library (its own vendored copy, with termwiz and wezterm-input-types pinned to one
  WezTerm revision) and a `windows-reactor` + Direct2D/DirectWrite front end, with no ConPTY and no
  local shell: the shell is the SSH server, and connection profiles (`profiles.json`) name the game's
  ports. It is faster than WezTerm and feels like a Windows application, and the owner will probably
  publish it beside the game as the Windows option. **D-21-23:** w11term is the engine's reference
  Windows client — kitty and sixel both reach it, so Windows players are not confined to sixel — and
  the help page names it first, WezTerm's software front end second, Windows Terminal (sixel) third.
  The ConPTY finding of §6 stands for every other Windows terminal.

## 13. Risks, and what this document refuses

- **No video codec.** A PNG a frame is the ceiling of this design, and it is enough: a flat frame
  at 640x480 is 7 KB. A busy textured scene is 50 KB (the MW2 frame), which is why the producers
  stay flat-shaded with fog and why the colour discipline is a rule.
- **No GPU, no meshes.** Software raycast, voxel-space and 2D rasterising; the owner's units are
  billboards and dolls, not models. This is a feature of the budget.
- **No alpha over text.** A picture is opaque and sits in its box; the text frame is around it, not
  under it. A translucent HUD would quadruple the colours and break the palette.
- **tmux.** Unsupported in v1: the probe is swallowed without passthrough, and passthrough plus
  unicode placeholders is a second kitty encoder. `help cockpit` says so.
- **The WezTerm OpenGL freeze** (Q-21-8) is a client bug the engine cannot fix; the software front
  end avoided it all evening.
- **A connection's pump is one task**, so one viewer cannot use more than one core and a slow
  client cannot stall the world — but a viewer's render does share the gateway's core with its SSH
  crypto. The step-down keeps that bounded; the launch box's vCPU count is the real limit.
- **The leash is not optional.** A view that lets a player look where the sim has not turned them
  must say so on screen (D-21-14's offset readout) or it misleads; G.4 is a correctness cut, not a
  feature.

## 14. Amendments owed (made by the cuts that land them)

- **Doc 02 §5.6** (capability detection): the active kitty probe, the DA1 attribute set, the
  encoder choice (G.1, G.2). *Made by G.1a and G.2.*
- **Doc 16 §5** (the cockpit): the pixel view as the cockpit's fourth surface beside line, TUI and
  web; the instruments as drawn (G.1). *Made by G.1a, in doc 16 §11.*
- **Doc 19 §5** (`+chart`): the chart as a picture (G.5). *Made at G.5c.*
- **Doc 20 §4** (the vehicle) and **§5** (knowing): the windscreen and the survey chart as pictures;
  the first map with height (G.7, Q-21-5).
- **Doc 17 §6** (the field pilot): the leash and the helm order (G.4). *Not made: doc 17's §6 is
  the due-event scheduling section, not the field pilot; G.4 is recorded here and in the cut notes
  until the owner names the section it belongs in (17e, the field pilot's acceptance, is a
  candidate).*
- **`docs/ops.md`**: `[pixel]`, `@pixel/stats`, the budget per viewer (G.1); the `assets/`
  directory deploy.sh ships and the sftp caps (G.11).
- **Doc 02 §5** (the SSH server): the `sftp` subsystem as the one subsystem the front door
  accepts, read-only, and the `XTVERSION` reply in the capability record (G.10, G.11).
