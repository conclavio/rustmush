# 22 — cut notes

One `## U.n` heading per cut of [22-unit-kit.md](22-unit-kit.md), in the order they land: what landed, what it deferred and why, how it was verified.

## U.0

The document (Draft 0.1, 2026-10-22), this file and the README row. The owner accepted the four proposed defaults (look separate from the rules pack, a 24x16 doll grid at most, four facings from `aspect` with eight reserved, the unit type with classification) and the build plan; asked afterwards whether the type should wait on a close reading as the chassis's section words do (16b cut Q.2), the owner chose the exact type at classification (D-22-5).

## U.1 and U.2

Built together (one commit), because the text doll and the team colours share the same renderers.

**U.1, the frame and team colours.**

- **The frame**: `arena::state::ChassisWords` (the per-unit table of chassis-static words, re-derived and never hashed) gains `unit_type`, the unit's definition id off `TacticalUnit::schema()`; `FrameContact` gains `unit_type: Option<SmolStr>`, set in `Arena::frame_scope` from the target's words **when the contact is classified** (`held.class.team()`), `None` before (D-22-5), and compared exactly by `FrameContact::same_as`; `Frame` gains the pilot's own `unit_type`. The `arena.cockpit` keyframe carries `team` and `unit_type` at the top (inserted after the big `json!` — the macro was at its recursion limit) and `unit_type` per contact. No frame version: fields added for the person in the seat are not one. The canonical fixtures are re-blessed (`cockpit_frame.json`, `view_frame.json`: BA and RB, classified, carry `tacsim.unit.line_tank`; X3 and X4 `null`).
- **Team colours**: `raycast::SpriteKind` is now `Red`, `Blue`, `Unknown`, `Wreck` (was `Hostile`, `Friendly`, …); `cockpit::contact_kind` reads the frame's `class` (the classified team), never `friendly`. Unknown is a neutral grey (was khaki), the lock's box and label a marker yellow (was hostile red). The text panel's contacts are coloured by team and a teammate is marked `friend`; the lock is yellow.

**U.2, looks and dolls.**

- **`mush_pixel::look`**: the look format (doc 22 §3) with its own check (`Look::parse`, which refuses unknown keys, an empty unit list, a doll over 24x16, ragged rows, a character the legend lacks, a legend character never drawn), `Doll::condition` (what is left of each part off the frame's `sections`: a named facet's armour, else the section's internals; destroyed is nothing) and `Doll::tones` (touching parts get different tones where they can, so two parts in the same state are told apart). `sprites` is accepted and unread until U.3.
- **Four shipped looks** (`crates/mush-pixel/looks/`): `tracked_tank.json` (the five line tanks, the heavy, the range target; a top-down grid with the hull's four facets as plates, the turret and barrel, the engine, the tracks), `wheeled_scout.json`, `biped.json` (upright, its left on the left: head, torso, arms, legs) and `drone.json` (a 10x5 body with its four facets). The haulers and the prospector are listed lookless with the reason (never in the arena); they get the section rows and no doll.
- **The text doll** (`cockpit_text::doll_rows`): the pilot's own look's grid in half-blocks, two grid rows to a text row (24x16 is 24 by 8), centred in the armour column, toned; replaces the hard-coded tank, so a biped pilot gets a doll.
- **The pixel doll** (`overlay::draw_doll`, the `Instruments` overlay): the same grid, a 3-pixel square a cell at scale, toned, each part outlined; a unit with no look gets a bar a section.
- **The contract test** (`mush-contract-tests/tests/unit_looks.rs`): every shipped unit type (fourteen, read from `crates/tacsim/content/units/`) has a look or is listed lookless with a reason; no unit is drawn by two looks; every unit a look names exists; every section of every unit it draws is in the doll; every doll part names a section, and a facet, each of its units has; every weapon short name is a weapon component of one of them.

**Verified.** `tacsim` lib (1,432) and `tests/cockpit.rs` (183; `i4_eight_watch…`, a load-sensitive test, failed once under the parallel run and passed alone), `mushd` `observer_rule` (32), `mush-bot` `arena_contract` (19), `mush-pixel`, `mush-net-ssh`, `mush-net-web`, `mush-ansi`, `mush-session` `resume`, the contract test, the web's vitest (655), clippy, rustdoc.

## U.3

The sprite sheets.

- **The format** (`mush_pixel::look::Sheet`, parsed by `Look::parse`): `size` (at most 64 a side), `size_cm`, `anchor`, a `palette` of one-character keys to **materials** (`team-dark`, `team`, `team-light`, `black`, `steel`, `pale` — a closed set, not free colours: see the budget below), `facings` (`front`, `rear`, `right` required, `left` optional; each a `still` frame, two to four `moving` frames and `muzzles` by weapon) and one to four `wreck` frames. Refused with a reason: a missing required facing, a row of the wrong width or a frame of the wrong height, a key the palette lacks, a material it does not know, a moving cycle outside two to four, an anchor or muzzle outside the frame, no wreck. `Sheet::frame` picks by facing (a missing left is the right mirrored), motion and step; `Sheet::wreck_frame` by facing.
- **Placeholder art** (`looks/tracked_tank.json` 32x16 at 7.2 m by 3 m: a side view with the barrel to the right, a front with the muzzle toward you, a rear with the engine grille, two-frame tread cycles, a wreck with the turret knocked askew; `looks/biped.json` 20x32 at 5 m by 10 m: front and rear with a four-frame walk, a side view with the gun arm forward and the legs scissoring, a wreck on its knees; `looks/unknown.json` 24x14: a grey hull with a question mark, the reserved unit id `unknown`). Drawn by a throwaway generator out of rectangles; the JSON is the source from here on.
- **The palette budget**: the raycaster's colours were 17 bases x 12 fog steps + 16 sky = 220. The three flat sprite bases (red, blue, unknown) are retired and the sprites get twelve **paints** — the three teams' dark, mid and light, steel, pale and the ground shadow — fogged in six steps (`SPRITE_FOG`): 14 x 12 + 16 + 12 x 6 = **256** exactly, pinned by a test. Black is the outline base, a wreck's body the old wreck base.
- **Drawing** (`raycast::draw_sheet`, behind the shared `draw_sprite`, so the raycaster and the heightfield producer both draw sheets): a `Sprite` carries an optional `SpritePose` (sheet, facing, moving, step); its box is the sheet's `size_cm` at its range with the anchor pixel standing on the ground under the contact; every pixel is hidden where the picture drew something nearer; a flat shadow ellipse about the anchor is drawn first over ground at about the unit's distance. The bracket rule is unchanged (no unit pixel shown, or under two pixels tall: the bracket, in its team's colour). `cockpit::contacts_as_sprites` gives each contact its look by `unit_type` (or the `unknown` look while unclassified), its facing from `aspect`, its motion from `moving`, and the picture's animation step (`CockpitPicture` counts draws; a step every three).
- **The contract test** also holds every sprite muzzle to a weapon component of the look's units, and the `unknown` look to drawing nothing else and having no doll.

**Measured** (release, the owner's box): the fixture cockpit at 640x256 with the HUD renders in 0.68 ms (0.65 before), PNG 4,060 bytes. `examples/fixture.rs` writes `sprites.png`, a line-up of the shipped sheets (a red tank side-on, a blue tank head-on, a blue biped striding toward you, a red biped walking left, an unknown, a wreck).

**Verified.** `mush-pixel` (the budget is 256; a sheet sprite is drawn in its team's colour, mirrored for its left face, with a shadow, a wreck without team colour; a sheet's check refuses what §3.2 says and picks its frames), `mush-net-ssh`, `mush-net-web`, the contract test, clippy, rustdoc. Not smoked live.

