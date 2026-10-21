# Pixel cockpit spike — notes (2026-10-21)

A first-person view of the arena streamed over SSH with the kitty graphics
protocol. Throwaway-quality by intent; this file is what it measured and what
it learned. The memory note `future-pixel-cockpit` holds the design argument.

## What was built

| piece | where | what |
|---|---|---|
| raycaster | `crates/mush-pixel/src/raycast.rs` | Wolfenstein-style DDA over the arena cell grid (`arena.map`: grass / water / road / open are floor, wall and woods block), floor cast per pixel with a faint cell grid, sky gradient, distance fog in 12 bands, contacts as tank-silhouette billboards placed by bearing and range, a per-column depth buffer. 320x220 RGB. Colours are a closed set (16 bases x 12 fog steps + 16 sky) so the PNG is always palette. |
| PNG encoder | `crates/mush-pixel/src/png.rs` | Palette (type 3) when <= 256 colours, truecolor otherwise; filter 0; zlib level 3 via the workspace's `flate2`; table CRC-32. |
| kitty framing | `crates/mush-pixel/src/kitty.rs` | The probe (`a=q` 1x1 image under id 31, then `CSI c`), a frame as `a=T,f=100,t=d,i=1,p=1,q=2,C=1,c=…,r=…` in 4096-byte base64 chunks inside `CSI ?2026h … CSI ?2026l`, and the delete. |
| key parser | `crates/mush-net-ssh/src/tui/keys.rs` | `ESC _ … ESC \` (APC) is swallowed; the probe's reply becomes `Key::GraphicsOk` / `Key::GraphicsErr`; `CSI ? … c` becomes `Key::DeviceAttributes`. Split bursts, BEL terminators and abandoned APCs are covered. |
| the view | `crates/mush-net-ssh/src/tui/pixel.rs` | Probe state machine (OK before DA1 = supported; DA1 first, an error, or 500 ms of silence = not), `^G` opens or closes, toggle-turn (`←`/`a`, `→`/`d`, space holds, `c` recentres) at 45°/s on a camera offset from the sim heading, a frame is sent only when its hash or its cell box changed, counters for the HUD. |
| wiring | `tui/app.rs`, `tui/mod.rs`, `server.rs` | The probe goes out after the first `@tui` frame; the output pump has a 50 ms frame clock that ticks the camera and paints only while the view is open; `@notui` and a close free the image; the HUD is two ratatui lines with the picture scaled into the cells between. |

The sim is untouched: the camera turns, the unit does not. The `arena.cockpit`
frame (version 6) already carries `x_cm`, `y_cm`, `heading_mdeg` and each
contact's `bearing_deg` / `range_m` / `friendly` / `class` / `reads_dead`, which
is everything the renderer needs — no schema change was required.

## Measurements

The canonical fixture scene (`web/src/lib/fixtures/cockpit`: the 96x64 yard,
the pilot at (65 m, 45 m) heading 045°, four contacts at 100–450 m), 320x220,
`cargo run -p mush-pixel --example fixture --release`, on the owner's Windows
box, 200-frame averages:

| step | release | note |
|---|---|---|
| render | 1.7 ms | the floor cast is the cost (70k pixels, a map lookup each) |
| PNG encode | 0.69 ms | 3.0 ms before the last-colour cache and the CRC table |
| PNG size | 2,090 B | 37 colours; the MW2 screenshot estimate of 49 KB was a textured worst case |
| kitty stream | 2,860 B | base64 + one chunk + sync; 4 ms of a 1 MB/s link |
| at 20 fps | ~57 KB/s per client | 0.5 % of the earlier per-client budget |

CPU: ~2.4 ms a frame → 48 ms/s per viewer at 20 fps, so about 20 viewers per
core on this box before the frame clock starts missing ticks. The skip-if-
unchanged check means a still camera with still contacts costs the render only
(1.7 ms) and no bytes.

## In real terminals (2026-10-21, later the same day)

The owner installed WezTerm (20240203) and Windows Terminal Preview 1.26, so
the frame was tried for real. The stream file is the example's `fixture.kitty`
with the `CUP` removed and the box set to 80x20 so a plain `cat` places it at
the cursor.

| path | result |
|---|---|
| `type fixture.kitty` in a local cmd under WezTerm | **nothing drawn** |
| `type fixture.kitty` in a local cmd under Windows Terminal Preview 1.26 | **nothing drawn** |
| `cat fixture.kitty` on conclav.io inside `wezterm ssh` (WezTerm's own SSH client) | **drawn correctly**, scaled into 80x20 cells: sky, grid, wall block, contacts |
| `type mw2.six` (sixel) in a local cmd under Windows Terminal Preview 1.26 | **drawn correctly** |

What that means:

- **ConPTY strips the kitty APC.** Anything that runs as a Windows console
  process — `cmd`, `ssh.exe`, PowerShell — has its output re-encoded by
  ConPTY, which drops `ESC _ … ESC \`. WezTerm's bundled ConPTY and Windows
  Terminal's both do. So on Windows, kitty graphics over SSH work only through
  a terminal's **own** SSH client that bypasses ConPTY (`wezterm ssh`, or a
  WezTerm SSH domain). A player typing `ssh` in a Windows shell will not get
  them, whatever the terminal.
- **Windows Terminal has no kitty graphics.** 1.25 added the kitty *keyboard*
  protocol (release notes #19817); the graphics request, microsoft/terminal
  #8389, is still open with a design spec from July 2026. It does speak sixel
  through ConPTY, and that worked first time.
- So **the sixel encoder is not the fallback, it is the Windows path**, and
  the second encoder to build. The web canvas is the third.

The frame-placement scaling (`c=…,r=…`) worked as intended in WezTerm: the
320x220 picture filled the 80x20 cell box.

Still owed: a live `@tui` → `^G` session against a running game (dev2 is
offered), watching for flicker on frame replacement in WezTerm and for the
probe reply arriving as marker keys rather than stray characters.

## What it looked like

The fixture PNG: a sky gradient to a pale horizon, green grass to the horizon
with a faint 10 m grid converging ahead, a grey wall block right of centre
(the yard's `#` run at 10 cells), and the four contacts as small dark
silhouettes near the horizon — BA at 450 m is two pixels tall, which is
physically honest at 320x220 with a 66° field of view and the reason a real
cut will want a zoom or a bigger sprite scale for the lock.

## Lessons and debts (for a real cut)

- **The frame clock ticks for every TUI connection**, open view or not; the
  cost is a lock and a bool at 20 Hz. A cut should arm the timer only while a
  view is open (a `Notify` or a `tokio::time::sleep` in the select).
- **The camera is on a leash but the sim is deaf**: a cut sends the turn as an
  order and interpolates the sim's pose between pulses, then reconciles the
  local offset against the sim heading when the pulse lands.
- **Toggle-turn is the v1 input**; hold-to-turn needs the kitty keyboard
  protocol's release events (`CSI … u`), which the parser does not yet decode.
- **The map is rebuilt from `arena.map` once per match** and cached; a map of
  512x512 open cells stands in until the feed arrives, so the view works
  before the pulled map lands.
- **Only the probe's reply is a marker**; replies to the frame image are
  suppressed with `q=2` and would be swallowed anyway.
- **Sixel and the web canvas are not here** — both are encoders of the same
  `Frame`; sixel at this frame density would be roughly 4x the PNG bytes
  before SSH compression, which is still under 15 KB a frame.
- Heightfield terrain (the MechWarrior horizon) needs a heightmap the arena
  does not carry; the Comanche-style voxel renderer is the cheap route.

## Live on dev2 (2026-10-21, evening)

The owner offered the unused dev2 instance on conclav.io. It now runs main as
a split (gateway + server) TacSim instance on a fresh store at rate 24, with
the `deploy/dev2` and `deploy/dev2-split` sets committed beside dev1's. dev1
was not touched. A test account `pixeltest` with a registered SSH key sat in
Red Pod 1 on a two-target range and drove the view from WezTerm's own SSH
client on the owner's Windows box.

What worked, first time, over the real path:

- the probe answered OK (HUD `kitty graphics OK`) and nothing of the reply
  leaked into the input line;
- `^G` placed the picture in the cell box at once, scaled to 133 by 46 cells
  of a 170 by 48 terminal (aspect-bound);
- toggle-turn swung the camera at 45°/s and the picture tracked it; no
  flicker on frame replacement in WezTerm; the match even resumed across a
  gateway restart and `BOARD` took the tank back.

Measured on the box (`top` over the two halves, `ss -ti` bytes on :6792) for
one viewer turning continuously, after the fixes below:

| what | value |
|---|---|
| frames on the wire | 20–21 a second (the HUD's measured `fps`) |
| render / encode a frame | 0.5–2.6 ms / 0.3–1.4 ms (release, 4-vCPU VPS) |
| PNG / wire a frame | 1.1–2.4 KB / 1.5–3.3 KB |
| bytes on the SSH channel | 24–44 KB/s depending on the scene |
| gateway CPU | 4–8 % of one vCPU |
| server CPU | under 1 % (the view costs the world nothing) |
| idle viewer (camera still) | 0 bytes, no writes |

So one viewer costs about a twentieth of a core and a 30 KB/s link, and the
earlier estimate of 20 viewers a core holds on this box.

Three spike debts the smoke found and fixed on the spot (commits after
1458a421):

1. **An idle tick wrote bytes.** The HUD printed a skipped-tick counter, so
   the text frame repainted twenty times a second; and a plain ratatui
   `draw` hides the cursor and repositions on every pass. On the first live
   run the client froze (below) and those small writes filled its 2 MB SSH
   window in twelve minutes, at which point the gateway's 5 s write timeout
   closed the connection. Now `pixel_frame()` returns nothing on an idle
   tick, pinned by the session test.
2. **Pictures went out off the clock.** Every event repaint (the arena's
   per-pulse cockpit frames) also emitted a picture while the camera moved,
   so the wire carried ~35 frames a second. Now only the frame clock sends.
3. **The `sent` total said nothing about rate.** The HUD reads a measured
   frames-per-second (frames in the last second).

The Windows-side incident: on the first `^G`, the owner's PC froze (mouse
and keyboard dead, caps lock still toggling: a GPU driver hang signature)
with WezTerm 20240203 on its OpenGL front end. Every later run used
`--config front_end="Software"` and was uneventful. Not root-caused; the
owner will investigate if it recurs. WezTerm's CLI (`wezterm cli send-text
--no-paste` against the GUI's `gui-sock-<pid>`) drove the pane without
synthetic keystrokes for the later runs.

Deploy gotcha for the record: a re-ship with `git archive` stamps files at
the commit time, which can be OLDER than the previous on-box build, so cargo
keeps the stale object; `touch` the shipped crate's sources before the
incremental rebuild.

## The load experiment (2026-10-21, later still)

The owner asked for 640x480 at 20 Hz and then the clock uncapped, to see
what the box can do given every resource. Two keys were added to the view
for it: `r` swaps 320x220 and 640x480, `u` swaps the 20 Hz clock for a
~1 ms wait. Same setup as above (dev2, one viewer in WezTerm's SSH client,
turning continuously, the box sampled for 14 s each run); the HUD's `fps` is
the frames of the last second.

| frame | clock | fps on the wire | render | encode | PNG | wire a frame | SSH bytes | gateway CPU |
|---|---|---|---|---|---|---|---|---|
| 320x220 | 20 Hz | 20–21 | 0.5–2.6 ms | 0.3–1.4 ms | 1.1–2.4 KB | 1.5–3.3 KB | 24–44 KB/s | 4–8 % |
| 640x480 | 20 Hz | 21 | 6.2 ms | 1.0 ms | 4.9 KB | 6.6 KB | 100 KB/s | 21 % |
| 320x220 | uncapped | **189** | 2.3 ms | 0.4 ms | 2.0 KB | 2.7 KB | 368 KB/s | 51 % |
| 640x480 | uncapped | **58** | 10.5 ms | 1.4 ms | 5.8 KB | 7.8 KB | 284 KB/s | 75 % |

(CPU is one vCPU of the 4-vCPU VPS; the server half stayed under 2 % in
every run. SSH bytes are after the transport's zlib, which takes roughly a
third off the base64 PNGs.)

What it says:

- **Uncapped, the render is the ceiling.** One connection's output pump is
  one task, so a viewer can take at most one core: 189 frames a second at
  320x220 is ~2.7 ms of gateway CPU a frame (render, encode, base64, the
  write), and 58 at 640x480 is ~13 ms. The raycaster's per-pixel floor cast
  is where 640x480 spends its time; a per-row ground lookup would roughly
  halve it, and that is a real cut's work, not the spike's.
- **At 20 Hz the budget is linear in pixels**: 320x220 costs ~5 % of a vCPU
  and ~35 KB/s a viewer, 640x480 ~21 % and ~100 KB/s. On this box that is
  about 80 viewers at 320x220 or 20 at 640x480 before anything else runs,
  which is why 320x220 scaled by the terminal stays the recommendation.
- **WezTerm's software renderer kept up with 189 frames a second** of
  kitty PNG replacements with no visible tearing in the captures; the
  picture was smooth to the eye at every setting.
- **The first 20 Hz run at 640x480 read 17 fps** because the clock was a
  sleep after each tick, so the 8 ms frame stretched the period; the pump
  now uses a deadline (`sleep_until`, resynced when behind) and 640x480
  holds 21.

The gateway logged one restart (NRestarts=1) after one of the evening's
full redeploys: the new process came up while the old one still held the
port and systemd's `Restart=on-failure` brought it back five seconds later.
Harmless here; a cut that redeploys often should make deploy.sh wait for
the port.

## Target (the owner's ruling, 2026-10-21)

**640x480 at 20 fps.** The budget it buys on the VPS is about 21 % of a
vCPU and 100 KB/s a viewer; the extra pixels over 320x220 are for the
instruments a real cut draws into the frame itself — minimap, heat gauges,
weapon recycle states, the armour-status doll — rather than as separate
placed images. The view's default is now 640x480 (commit 8732d107) and `r`
steps down to 320x220 for a slow link; dev2 keeps the 320-default build
until it is next redeployed.

## The instruments (2026-10-21, the owner's next ask)

Drawn into the frame after the raycaster, from the same `arena.cockpit`
keyframe and `arena.map` the TUI's panes read (`mush_pixel::overlay` draws,
`mush_pixel::cockpit` reads the JSON; the SSH view and the fixture example
share both):

- **minimap**, bottom-left: a 60-cell window around the pilot, north up,
  ground coloured by cell, the pilot a white dot with a heading line,
  contacts coloured by kind, the named spots as letters;
- **heat**, bottom centre: a bar green to amber to red, `OVERHEAT` when the
  frame says so, `HEAT --` for a hull with no heat model (dev2's stock tanks);
- **weapons**, top right under the status strip: a row each — name, a
  recycle bar (full when ready, the countdown while it cycles), rounds
  coloured by what is left, the hit chance when there is a lock;
- **the armour doll**, bottom right: a top-down tank — hull plates front,
  rear, left, right and top, the turret with its barrel, the tracks as bars
  beside the hull, the engine as a block at the rear — each green, amber,
  red or gone by its armour or internals;
- **the status strip**, top left: callsign, speed, camera heading, power,
  the lock and its range.

A 5x7 bitmap font (capitals, digits, a little punctuation) at 1x for
320x220 and 2x for 640x480; everything opaque from a fixed set of colours,
and a test holds the whole frame under the palette PNG's 256 at both sizes.
Live on dev2 at 640x480 the instruments add ~2 ms of render (7.6–8.6 ms a
frame) and ~1.5 KB of PNG (6.6 KB; 8.9 KB on the wire) and the clock holds
21 fps.
