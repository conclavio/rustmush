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
