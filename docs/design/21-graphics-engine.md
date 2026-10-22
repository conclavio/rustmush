# 21. The graphics engine — one frame, three encoders

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
| The orbital view — the system, the lanes, your dot from Phobos to Ceres (doc 19) | `sky.chart` (already pushed; the TUI's screen `c` and the web's chart tab draw it as text) | the chart as a picture: bodies, orbits, the plan's legs, the ship's dot, light-lag rings |
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

The frame is placed as **one image** under one fixed id and placement, scaled by the terminal into
the cell box the view computes from the terminal's size and the frame's aspect (the HUD line above,
the key legend below). Instruments live inside the frame (the owner's ruling), so there is one image
to replace a tick, one id to free, and no z-order to reason about. A bucket-C form that embeds a
bucket-B scene places it the same way, as one image beside the text.

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
same step-down.

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
| **G.1 the engine from the spike** | `mush_pixel` promoted: the producer seam (`Picture` = producer + overlay + clock interval), the step-down on back-pressure (§3.4), `[pixel]` config, `@pixel/stats`, the floor-cast per-row lookup, the `u` key behind a privilege, the probe's attribute set | M | — | 640x480 render ≤4 ms on the VPS; a client that stops reading costs one frame; the step-down test; the contract gates |
| **G.2 sixel** | the sixel encoder from the same frame; DA1 attribute 4 detection; the 10 Hz default; the encoder choice (§4.4); smoked in Windows Terminal over `ssh.exe` | M | G.1 | the MW2 fixture frame and the cockpit frame byte-compared against a reference encoder; the live smoke in Windows Terminal |
| **G.3 the web frame** | the gateway's PNG over the websocket as a binary message; the cockpit tab's canvas; the same clock and hash | S | G.1 | the web cockpit shows the picture the TUI shows, pinned by a fixture test |
| **G.3b the web render** | `mush-pixel` to WebAssembly; the client renders from its own feeds; the socket frame as the fallback | M | G.3 | the fixture renders pixel-equal in the browser and the TUI; zero gateway CPU for a web viewer |
| **G.4 the leash and the keys** | the turn as a helm order through the control channel; interpolation between pulses; the kitty keyboard protocol for hold-to-turn | M | G.1 | the sim's heading and the camera's reconcile within one pulse; a release event stops a turn |
| **G.5 the scene model and the orbital view** | `scene` (§3.3) and its rasteriser; the first builder: `sky.chart` as a picture with the plan's legs, the ship's dot and light-lag rings; the minimap re-based on `scene` | L | G.1 | `+chart` draws the picture in every encoder; the minimap pixel-equal before and after |
| **G.6 the doll and the fitting screen** | the armour doll as a standalone bucket-B picture with a legend; the fitting paper doll with hardpoints and arcs embedded in the sheet (bucket C) | M | G.5 | the fixture doll; the fitting screen on the TUI and the web |
| **G.7 the heightfield** | the voxel-space producer; the first map with relief (Q-21-5) | M | G.1, a map with height | a crater rim on the proving ground or a Ceres site at ≤6 ms a frame |
| **G.8 debrief and replay** | the replay document (16b I.2) as a bucket-B picture with a scrubbable timeline | M | G.5 | `SIM REPLAY` draws the match; `,`/`.` scrub it |
| **G.9 the boards** | the strategic board and the investigation board; the player annotation store they share | L | G.5, an annotation store | two players see each other's pins |
| **G.10 bells and sounds** | the cockpit's notice kinds as sound names (`contact-gained`, `contact-lost`, `hit`, `overheat`, `power-online`, …; 1–64 of `[A-Za-z0-9_-]`); BEL for every terminal; **w11term's OSC 440, wire version 1** — `ESC ] 440 ; 1 ; play ; <name> ST` and `ESC ] 440 ; 1 ; pack ; <pack_version> ST`, four fields, no options, no loop (w11term's `docs/game-client-protocol.md`; mintty's older `440;file` form is NOT this and mintty gets BEL) — only for a terminal whose `XTVERSION` reply names w11term; kitty's OSC 99 with its `s=` sound for kitty; the client plays serially at a 40 ms floor with a 16-deep queue and ≤5 s clips, so an alarm is the server re-sending its name on a cadence, debounced server-side at one sound a kind per two seconds; `@sounds on|off`; **`XTVERSION` joins the `@tui` probe, which first needs the TUI key parser to swallow DCS (`ESC P … ESC \`) the way it swallows APC** — today that reply would decode as F1 plus typed text (D-21-24) | S | G.1 | a contact gained rings in w11term and in Windows Terminal's bell; the overheat alarm repeats while the frame says so and stops when it clears; the sound names are the notice kinds; a DCS reply never reaches the input line |
| **G.11 assets over the player's own connection** | the gateway's SSH server accepts the `sftp` subsystem (russh-sftp, **protocol v3**) over a READ-ONLY virtual tree rooted at a shipped `assets/` directory — open/read/close/opendir/readdir/stat/lstat/fstat/realpath only, paths canonicalised, a size cap a file and a bytes-a-second cap a connection; if it advertises `limits@openssh.com` its `read_len` is ≥ 16384 (the client reads 16 KiB and refuses larger-than-limit reads); `assets/manifest.json` as w11term's protocol defines it — schema `version` 1, `pack_version` an OSC-grammar token, `files[{name, path, kind, sha256, size}]`, every `size` > 0, names and the pack version 1–64 of `[A-Za-z0-9_-]`, sound names never `system`/`off`, paths of two or more segments with no `.`/`..`, `.wav` lower-case, no unknown fields (the schema is frozen at 1) — and the Hello's `asset_pack {version, pack_version, manifest}` for the web client; the gateway announces the pack once after login and once after a deploy (an announcement during a running sync is dropped by the client, so re-announce on a slow cadence, never per frame); the first pack stays small (sounds only, well under 10 MiB: sequential 16 KiB reads fetch ~25 MiB in the client's 180 s at 100 ms RTT); w11term syncs what changed into its per-host cache and OSC 440 names resolve there first; the sprite sheet, a font with the HUD's glyphs and the help topics ride the same manifest (D-21-25) | M | G.10, deploy.sh shipping `assets/` | w11term fetches the sound pack on first connect and plays G.10's sounds from it; a write, a rename or a `..` is refused; the manifest version in the Hello changes when the pack does |

## 11. Decisions taken here

- **D-21-1** The picture is a fourth renderer of the per-observer feeds, rendered in the gateway
  half per connection; nothing pixel-shaped crosses IPC; the server stays pixel-free.
- **D-21-2** Kitty: inline PNG under one id and placement, active probe, APC replies as marker keys.
- **D-21-3** Sixel: the same frame as a whole-picture DCS stream, DA1 attribute 4 detection, 10 Hz
  default.
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
- **D-21-10** One image a frame, instruments inside it, placed into the cell box by aspect.
- **D-21-11** 640x480 at 20 Hz is the target and the budget is a quarter of a vCPU and 110 KB/s a
  viewer; twenty bucket-A viewers a vCPU after G.1.
- **D-21-12** Both terminal encoders ship early because Windows needs sixel and WezTerm needs kitty.
- **D-21-13** Toggle-turn first; the kitty keyboard protocol for hold keys; mouse later.
- **D-21-14** The camera on a leash: a turn is a helm order, the view interpolates, the picture
  never claims what the sim has not granted.
- **D-21-15** The heightfield producer is built when the first map carries height.
- **D-21-16..22** The Q-21-1..7 defaults, accepted by the owner 2026-10-21: sixel at 10 Hz; an
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
  encoder choice (G.1, G.2).
- **Doc 16 §5** (the cockpit): the pixel view as the cockpit's fourth surface beside line, TUI and
  web; the instruments as drawn (G.1).
- **Doc 19 §5** (`+chart`): the chart as a picture (G.5).
- **Doc 20 §4** (the vehicle) and **§5** (knowing): the windscreen and the survey chart as pictures;
  the first map with height (G.7, Q-21-5).
- **Doc 17 §6** (the field pilot): the leash and the helm order (G.4).
- **`docs/ops.md`**: `[pixel]`, `@pixel/stats`, the budget per viewer (G.1); the `assets/`
  directory deploy.sh ships and the sftp caps (G.11).
- **Doc 02 §5** (the SSH server): the `sftp` subsystem as the one subsystem the front door
  accepts, read-only, and the `XTVERSION` reply in the capability record (G.10, G.11).
