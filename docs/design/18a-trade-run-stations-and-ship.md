# 18a — The trade run's places: five stations and the Dray's interior

**Revised with doc 18 Draft 0.4 (2026-10-17), as built through IC12.** The packs this brief describes have
shipped: `region.ceres_freeport` and `region.stickney_yards` at v1, `neighborhood.ceres_freeport` at **v2**
and `neighborhood.stickney_yards` at **v3** (IC1, with Stickney's v2 in IC8 and both re-publications in
IC12), and the Dray's interior in `tacsim.pack.dray` **v2** (IC8). The brief's prose is kept as written;
where a shipped pack differs, an **"As built (IC*n*):"** note beside the text says how and where, in doc
18's style ([18-trade-run-vertical-slice.md](18-trade-run-vertical-slice.md); the per-cut record is
[18-cut-notes.md](18-cut-notes.md)).

**Extended with doc 19 cut A.8 (2026-10-19).** Three stations join the two: Aitken Highport on Luna
(§5), Voltaire Anchorage on Deimos (§6) and Ligeia Terminal on Titan (§7), each a region pack and a
neighborhood pack at **v1** (`region.aitken_highport` + `neighborhood.aitken_highport`,
`region.voltaire_anchorage` + `neighborhood.voltaire_anchorage`, `region.ligeia_terminal` +
`neighborhood.ligeia_terminal`), named by lane v8's `luna_port`, `deimos_port` and `titan_port`
([19-astronomy-engine.md](19-astronomy-engine.md) §3.8). §1 and §2 were written before their packs; §5–§7
are written from packs already authored and validated, so the packs are the authority and these sections
index them — the theme, the overview, the rooms with their short descs and detail keys, the listings, the
cast, the lore pool's subjects, the boxes and the exit words. Each room's full desc and detail text is the
pack's own and is not repeated here. The new names await the owner's "names ok" (§4).

**Content brief, Draft 0.1 — 2026-10-15**, the companion of
[18-trade-run-vertical-slice.md](18-trade-run-vertical-slice.md) Draft 0.2 (§4.5, §4.6, D-18-35). It
is the room-by-room text of Ceres Freeport, Stickney Yards and the Dray-class hauler's interior, with
the NPC cast, the lore pools and the ambience, written for the pack author. Every name below doc 11's
principals is binding under R8 (Q-18-12, "names ok", owner 2026-10-15); every number is bound by the
decisions of doc 18 §7, themselves binding under R8;
every tree claim is file:line at `8a32734a` (or at `c87d78f7`, IC0 landed, where marked). The two
flavour briefs the second review produced are carried here faithfully; where the review's judges
disagreed on the room lists the reconciliation is stated in §0.3. Revised the same day with the
Draft 0.2 review fold (doc 18 §10 lists the findings): the Exchange's own factor and the commissary's
storekeeper (thirteen NPCs; every listing signed by its own house), `+ship/leave` in the airlock only,
the laid-up hull names, the airlock forward, Ceres' spun ring, IC time only in room text and NPC
speech, and the one fiction for the sailing plan (the port executes the captain's signed plan; the
ROM only flies). **Revised with doc 18 Draft 0.3** (the owner's second-round rulings R7–R9): the
Dray's hold reads tonnes **and** cubic metres against both its limits (100 t, 120 m³) in the Cargo
Hold, the Central Passage's slate and the Bridge; the Yard's crated fittings (`industrial_parts`) are
sold from the `YARD — SPARES` cage, signed by the Stickney service clerk and wanted by the Co-op at Ceres
(a sixth signatory); Kasei bids 280 for concentrate; and every passage timing in a room or a mouth is
"in a good season", because the sky will move after the slice (doc 18 §4.8).

## 0. Purpose, and how this brief becomes packs

### 0.1 What the brief is for

Doc 18's loop and the moon ports of doc 19 cut A.8 touch sixty-seven rooms: thirteen at Ceres, twelve at
Stickney, twelve at Aitken Highport, eleven at Voltaire Anchorage, twelve at Ligeia Terminal, and the seven
aboard a Dray. The five stations place fifty-two NPCs: thirty-four cast (Ceres 7, Stickney 6, Aitken 9,
Voltaire 6, Ligeia 6), fifteen of them Warrant signatories — the NPCs the lane's port entries name (Ceres 3,
Stickney 3, Aitken 4, Voltaire 2, Ligeia 3) — and eighteen principals placed `enabled:false` (Ceres 6,
Stickney 3, Aitken 4, Voltaire 3, Ligeia 2). A pilot who follows R3 lives in them for a 30–60 minute
session a day and sleeps aboard the rest; the rooms must be places before they are desks. This brief gives each room a purpose in the loop, a
short desc and a full desc in the setting's voice, the `look` details that reward a second glance, its
exits with aliases, its items and boards, and its access; each station gets a theme, an overview, a cast
with voices, knowledge bounds and non-LLM fallbacks, a lore pool in three secrecy tiers, and ambience
lines. The builder of cut IC1 authors the four atlas packs from §1 and §2; the builder of IC2 and IC4
authors the Dray's interior template from §3; cut A.8 authored the six moon packs from its design's §4,
and §5–§7 record them.

### 0.2 The pack shape

- **One region and one neighborhood per station**, mush-content atlas packs: `region.ceres_freeport` +
  `neighborhood.ceres_freeport`, `region.stickney_yards` + `neighborhood.stickney_yards`. Each region is a
  **surface** region on its body (`ceres`, `phobos`), the tested path (`proving_ground.json`,
  `region.limbo`); no pack or test uses an orbital region. Each neighborhood has one site and one
  structure.
- **Decks are structure floors.** Floors rise in index and `z_offset_m` (`mush-atlas/src/validate.rs:
  1679-1715`). Ceres: Freight Deck 0 at z 0 (6 m), Public Deck 1 at z 6 (4.5 m), Office Deck 2 at z 10.5
  (3.5 m). Stickney: Freight Deck 0 at z 0 (7 m), Public Deck 1 at z 7 (4.5 m). Rooms on a floor are
  `{floor:[structure,i]}` and lie inside the structure (`apply.rs:1113-1127`). A cross-deck exit carries
  `portal.at` (a segment inside both rooms) and `dz_m`, negative going down; there is no `lift` mode, so
  lifts and stairs are `mode: stair` and hatches `mode: hatch`, the Registry ladder `mode: ladder`.
- **Every room is `kind indoor` or `circulation`**, never outdoor (or the applier forces OUTDOORS and
  weather follows, `apply.rs:1088-1090`). Descs are present tense, no "you see", 60–120 words, no named
  NPCs in a room desc; details are one or two dry sentences with a fact in them, at most 32 per room of
  2 KiB each (`apply.rs:75-79`). Exit aliases avoid the kernel words `l i inv ex go move take p`
  (`apply.rs:1366-1372`) and never `board`, `disembark` or `dock` (`disembark` is the sim's egress alias,
  `mush-sim/src/commands.rs:3428` at `c87d78f7`; exits match before commands, `cmd.rs:22-26`). Exit
  matching is exact over `;`-segments and aliases (`mush-core/src/commands/movement.rs:24-28`), so
  **every exit word typed in doc 18 §2 and §6 is an alias authored here**, and IC12's story test walks
  those paths against the packs; a path doc 18 types that this brief does not author is a defect in
  one of the two.
- **Locks.** A paired exit locks only `names[0]`, the way in (`apply.rs:1500-1519`); `staff` expands to
  `flag^WIZARD|flag^ROYALTY`. The one restricted room below the office deck at each station takes `lock:
  staff` and `portal.visibility: hidden` on the way in (the staff-lobby precedent) and `access:
  restricted`; a room reached only through a locked exit must be restricted or it fails
  `topology.unreachable`, and an unlocked way from public ground into it fails `topology.private_ingress`
  (`validate.rs:2015-2072`). No berth-marker locks ship in the slice (the freight rooms that needed them
  are P-F, §0.3).
- **Walls.** Only structure rings and `wall` features are walls on the scene map (`raster.rs:412-450`);
  two adjacent public rooms with no exit between them are open floor on `+map` unless the partition is a
  `wall` feature with `blocks.movement`. The partitions listed per station in §0.4 are authored as walls;
  door portals cut them. Counters, windows and rails are `feature` entries too; look text goes in
  `RoomDef.details`, never in a feature.
- **Fixtures are not items.** An unlocked pack item can be taken by anyone (`get` reads a Thing's Basic
  lock as its take lock, `mush-core/src/commands/items.rs:46-52`; `ItemDef` has no lock, `def.rs:509-532`,
  and the atlas applier sets none, `mush-atlas/src/apply.rs:1205-1262`). Boards, maps and windows are details; the only `item` entries are the
  storefront containers that back the vendor vaults, placed in each station's staff-locked Maintenance
  Spine where no player can reach them.
- **Markers.** The Visiting Berth Gallery at each station carries `markers: ["port.dock"]` (D-18-19):
  the validator's topology seed for a room reached by carrier, and the presentation room of the port's
  passive domain. Without it every public room at Stickney, which has no foot route, fails
  `topology.unreachable` (`validate.rs:1958-2000`). Ceres also has the unlocked Limbo door.
- **Themes.** `Theme` fields: `name`, `style_guide`, `vocabulary`, `banned`, `exemplars`, `desc_length`,
  `pov_rules` (`theme.rs:80-99`); nearest tier wins for scalars, lists union. The stations are
  separate regions with no common parent below the staff-set world theme, so the 2350 base (banned
  words, pov rules, desc length) repeats in each region pack and the flavour sits on each neighborhood.
  Theme has no `deny_unknown_fields`: a misspelled field name is dropped, a wrong type fails decode.
  Banned words only warn.
- **Hydration** `{"enabled": false, "max_player_sites": 0}` on every neighborhood (the serde default and
  the staff lobby's choice). Nothing else auto-expands.
- **NPCs** ship inside the pack that makes their rooms (`mush-npc/src/content.rs:214-219`) with
  `interfaces` naming only providers that exist (atlas, travel, help, market), a `fallback` list, a
  persona, and lore tags. Principals are placed `enabled:false` so the faction cut binds existing objects.
- **The port entry** (doc 18 §4.7) names rooms by `<pack>.<entry>` and resolves them through the latch
  (`travel.rs:960-967`); the applier refuses before any mutation if a named room lacks `RoomGeo` or the
  gallery lacks `port.dock`. The entry ids used below are the ones doc 18's `port` JSON references.
- **Tests.** A copy of the starter digest-pin test (`crates/mush-atlas/tests/starter.rs:31-60`), a copy
  of `the_starter_packs_validate_clean` (`tests/validate.rs:136-152`), and a kept-world boot test; boot
  packs bypass the validator and a refused first apply aborts dev1's boot (`module.rs:113-139`).

**As built (IC1, IC8, IC12):** the packs ship as described, at `neighborhood.ceres_freeport` v2 and
`neighborhood.stickney_yards` v3 (the region packs at v1), each version re-published whole with its digest
re-pinned (`crates/tacsim/tests/stations.rs`, `PINNED`; `crates/mush-server/tests/boot_parity.rs`'s
kept-world test). The packs are strict (`deny_unknown_fields`), digest-pinned and applied once under the
content latch; a kept world takes a new version only through `@atlas/pack plan` then `apply`, never at a
boot (`crates/tacsim/tests/stations.rs::ic12_a_world_latched_at_the_old_station_versions_takes_the_new_through_plan_and_apply`).
**A detail name admits only `[A-Za-z0-9_ ]`** (`check_details`, `mush-atlas/src/apply.rs:1012`), so a
hyphen or an apostrophe in a detail key is refused: the five details below that had them ship under plain
keys — `hull registry board` (C09), `sailing plan forms` (C14), `sailing plan pad` (P05), `yard time board`
and `insurers mark` (P11) — and the Dray's `builders plate` and `tie points` likewise (IC1's lead ruling:
rename, never widen the validator). **The validator** (`mush-atlas/src/validate.rs`) now reports a room's
`short_desc` over 256 bytes and a neighborhood's `ambience` over 24 lines, blank, or over 512 bytes a line
as `budget.text` errors, so a pack fails validation rather than the apply, and `topology.unreachable`'s hint
reads "join it to a reachable room with an unlocked exit, mark it `port.dock` if ships reach it, or make it
restricted" (IC12). No detail in either pack carries a live market figure or an engine word, and no Ceres
detail names a verb (`crates/tacsim/tests/stations.rs::ic12_the_five_details_ship_under_plain_keys`
checks for a `+` before a letter anywhere in the text); Stickney's details still name `+atlas` (P04's
`shift boards`) and `+market/list` (P07's `intake board`, P08's `price board`), an unowned residual;
fiction figures stay (IC12 O3).

**As built (A.8):** the three moon stations take the same shape at v1. Each region pack holds one
**surface** region on its body — `region.aitken_highport` on `luna`, `region.voltaire_anchorage` on
`deimos`, `region.ligeia_terminal` on `titan`, extent [[0,0],[400,400]], the 2350 base theme repeated —
and one `locality.<location>` of kind `port`; Aitken and Ligeia are orbital in the fiction and authored on
the tested surface path all the same. Each neighborhood has one `site.transit` site and one structure of
two floors: Aitken's Hub Deck 0 at z 0 (6 m, unspun) and Drum Deck 1 at z 6 (4.5 m, spun at a sixth of a
g); Voltaire's Tank Deck 0 at z 0 (7 m) and Roads Deck 1 at z 7 (4.5 m); Ligeia's Cargo Deck 0 at z 0
(7 m) and Company Deck 1 at z 7 (4.5 m). Each Visiting Berth Gallery carries `port.dock` and seeds its
station's topology (no moon station has a Limbo door); each station has one restricted spine behind one
`lock: staff`, `visibility: hidden` hatch, holding the storefront items and the principals `enabled:false`.
The neighborhood themes' banned lists add `shields`, `stardate` and `hangar bay` to the region base. No
detail names a verb or carries a money figure; the only numerals in moon room text are years and the
Voltaire lobby placard's step numbers. Every exit
word is drawn from the two shipped stations' alias set plus each station's own nouns and codes, no alias
sits on two exits of one room, and `exit_command_words` is the gate (§5.7, §6.7, §7.7).

### 0.3 The room lists, reconciled

The owner's pasted proposal drew fifteen rooms per station. The second review's three judges endorsed
different lists: the daily-loop and buildability lenses pruned the three freight-side rooms per station
behind berth-marker locks (Ceres C11 Freight Transfer Junction, C12 Bonded Freight Hall, C13 Tank Service
Gallery; Stickney P06 Bonded Freight Hall, P13 Freight Transfer Junction, P14 Tank Service Gallery) to the
post-loop freight-deck cut P-F, because each needs berth-holder marker locks, marker sync from docking
records, boot reconcile, the `+move` restricted fix (`window.rs:2276-2287`) and `ExitDef` fail text, and
the loop never walks them (cargo moves between vaults); the fidelity lens kept all fifteen and placed
the principals on an office deck. This brief takes the pruned lists **and** the office deck: Ceres ships
thirteen rooms (C01–C10, C14, C15 and the Port Authority Gallery C16, where Wen, Varga, Ndiaye,
Osei-Ferrante and Holm are placed `enabled:false`), Stickney twelve (P01–P05, P07–P12, P15, with
Castellanos, Tanaka and Bask placed `enabled:false` in the Yard Maintenance Spine). The pruned rooms stay
visible as background through the dockmaster's window, the receiving office's barred window and the
spines' stencils, so P-F adds doors, not places. Also reconciled: Ceres C09 is the Tycho Mutual Agency
(the pasted Hull Brokerage is withdrawn under R1, since no hull is sold at the start); Tycho's Phobos
agent shares P11 rather than taking a room; the Co-op's programme desk shares C05 with the Registry
counter (the fiction lens's separate Co-op Hall is folded in, because every room must have a verb or a
scene).

**As built (IC1):** the lists ship as reconciled here: Ceres 13 rooms, Stickney 12.

**As built (A.8):** the moon lists are the A.8 design's and ship as drawn: Aitken Highport 12 rooms,
Voltaire Anchorage 11 (the Stickney boxes, deliberately), Ligeia Terminal 12. Each station has one social
room on the Slack Line and Shift Board precedent — the Cold Trap, the Long Straw, the Ninety-Four — with a keeper NPC.

### 0.4 Geometry (metres, neighborhood frame; the pack author's starting points)

**Ceres**, neighborhood footprint [[100,90],[220,200]], site "Freeport Hub" [[110,100],[210,190]],
structure the same. Public Deck (floor 1): C05 [110,165]–[140,190]; C06 [140,165]–[175,190]; C09
[175,165]–[210,190]; C03 [110,140]–[140,165]; C04 [140,140]–[175,165] (circulation); C07 [175,140]–
[210,165]; C02 [110,125]–[140,140]; C10 [140,125]–[175,140]; C08 [175,125]–[210,140]; C01 [110,110]–
[210,125]. Freight Deck (floor 0): C14 [110,140]–[140,165]; C15 [140,140]–[175,165] (restricted, under
C04). Office Deck (floor 2): C16 [110,165]–[175,190] over C05/C06. Cross-deck: C03↔C14 public stair at
(125,150), dz −6; C14→C15 hatch on their shared edge (staff, hidden); C05↔C16 ladder at (125,180), dz
+4.5 (staff, hidden). Wall features on the exitless public seams, all six of them: C02|C10 (x=140),
C08|C10 (x=175, y 125–140: the Chandlery's wall onto the Slack Line), C01|C10 and C01|C08 (y=125),
C05|C06 (x=140), C06|C09 (x=175). Budget target 13, maximum 16.

**Stickney**, neighborhood footprint [[100,80],[210,190]], site "Yard Rim" [[110,90],[200,180]]. Public
Deck (floor 1, z 7): P05 [110,155]–[132,180]; P08 [132,155]–[155,180]; P11 [155,155]–[177,180]; P12
[177,155]–[200,180]; P03 [110,130]–[125,155]; P04 [125,130]–[185,155] (circulation: the one wide
corridor along the rim, 60 m, so that it shares an edge with every desk it opens onto — P05 on
x 125–132, P08 on 132–155, P11 on 155–177, P12 on 177–185); P09 [185,130]–[200,155]; P02 [110,115]–
[125,130]; P10 [125,115]–[185,130]; P01 [110,100]–[200,115]. Freight Deck (floor 0): P07 [110,150]–
[140,180]; P15 [140,150]–[170,180] (restricted, under P08). Cross-deck: P08↔P07 public stair at
(136,165), dz −7 (inside both P08 and P07); P07→P15 hatch on their shared edge (staff, hidden). Walls,
all seven exitless seams: P05|P08 (x=132), P08|P11 (x=155), P11|P12 (x=177), P03|P05 (y=155,
x 110–125), P09|P12 (y=155, x 185–200), P02|P10 (x=125), P01|P10 (y=115, x 125–185). Budget target
12, maximum 16. Stickney has no Limbo door; P01's `port.dock` seeds its topology.

**The adjacency rule, checked, not trusted.** A plain `door` between two footprints that share no edge
is `geometry.portal_gap`, an **error** (`validate.rs:1808-1840`), and the applier refuses the exit
outright ("the footprints of A and B do not touch; give portal.at or portal.distance_m",
`apply.rs:1453-1457`), which on a boot pack aborts the first apply; and two public rooms that share an
edge with no exit between them are open floor on `+map` unless walled. The boxes above were cut so
that every door shares an edge and every exitless seam is in the wall list (the first draft's P04
[140,130]–[170,155] touched neither P05 nor P12, and five seams were unwalled); IC1's validate-clean
test runs an adjacency check over the authored boxes so the next re-box cannot regress it.

**As built (A.8):** the moon stations' boxes, cross-deck points, hatches and walls are in §5.7, §6.7 and
§7.7, read from the packs. The validator skips geometry for an exit with `portal.at`, so each stair's and
ladder's point was placed inside both its rooms and outside the spine by hand (A.8 design §4.1).

---

## 1. Ceres Freeport

### 1.1 Theme (`neighborhood.ceres_freeport`; the 2350 base on `region.ceres_freeport`)

**Name.** TacSim 2350: the Freeport.

**Style guide.** Ceres Freeport, 2350: a commercial port built around freight. Bright wayfinding is
bolted onto worn pressure bulkheads; pump vibration comes up through the deck plates on the freight side
and is felt, faintly, everywhere; water handling dominates the working half of the port and the
Freeport's power shows as paperwork and access. Hard-ish SF (doc 10 §3): fusion power and propulsion,
thrust and spin for weight, no energy shields, no reactionless drives; compute scarce, automation
excellent and never in authority. Governments sound institutional, corporations sound like brands (doc
11 framing). Rooms are written present tense, second person absent (no "you see"), 60–120 words, no
named NPCs in a room desc; each room distinct in sound, light and smell. **Weight:** Ceres pulls three
hundredths of a g, so the Freeport Hub is a **spun ring** moored to the Occator terminal and the
playable decks ride its rim at about a third of a g — enough that taps drip, loose chairs stay put,
people lean on rails and nobody clips on; the cradles beyond the gallery glass and the freight tunnels
are at Ceres' own pull, where everything is clamped (doc 10 §3: no artificial gravity beyond thrust and
rotation; Stickney, by contrast, is written for Phobos's nothing, §2.1). Texture: pressure doors with
inspection stickers, cooperative notices three deep, patchwork cargo stencils, Registry forms, chits, cr
in major units (7.00cr, never 700). Time is IC in every desc, detail and NPC line: the campaign's
compression (doc 10 1A) is an OOC knob that only the `+ship` renders and the `look` status line
speak.

**Vocabulary.** deck, berth, lock, pressure door, bulkhead, manifest, tare, reaction mass, volatiles,
chit, Port Watch, Registry, Clearing, Co-op, the lay, stake, bonded, pump, manifold, sailing plan, loading
schedule, spinward, dry money, inspection sticker, cooperative notice, Kirkwood, Stickney, cr.

**Banned.** warp, hyperspace, FTL, teleport, alien, android, gravity plating, shield, stardate, you see,
hangar bay.

### 1.2 Overview

Ceres Freeport is the Belt's commercial port and the start of the trade run: the place that holds the
water, the propellant, the Belt's bank and the Registry's Belt mirror, and has no army at all (doc 11
§3.3, "institutional in name, mercantile in practice; rich; unarmed"). It is a freight port first.
Everything a visitor walks through is bolted onto a working water-and-propellant terminal: the public
rooms are bright, signposted and worn, the freight rooms behind them are loud, and the Port Council's
power is exercised, as doc 11 puts it, as paperwork. The Freeport holds the station de jure and de facto
through the Port Watch; its docks are run in practice by the people who work them, and the Belt
Squadron's cruiser sits at a berth nobody on the public deck can see.

Physically the playable port is one structure, the Freeport Hub, on three decks. The Public Deck (floor
1, +6 m) is a grid of three rows: the Registry & Co-op Desk, the Ceres Clearing Office and the Tycho
Mutual Agency across the top; Port Watch Arrivals, the Freeport Concourse and the Exchange Hall as the
circulation spine through the middle; the Pressure Lobby, the Slack Line and the Co-op Chandlery along the
bottom, with the Visiting Berth Gallery south of the Pressure Lobby as the only way to a hull. Adjacent
rooms without a door are solid bulkhead, authored as wall features so the map agrees. The Freight Deck
(floor 0) holds the Dockmaster's Office, reached by the public stair from Arrivals, and the staff-locked
Maintenance Spine behind a hidden hatch, where the vendors' storefront manifolds physically sit; the
bonded hall, the transfer junction and the tank gallery are visible background through windows and
board lines until freight-deck cut P-F. The Office Deck (floor 2) is one restricted room, the Port
Authority Gallery, reached by a hidden staff ladder from the Registry, where doc 11's principals (Wen,
Varga, Ndiaye, Osei-Ferrante, Holm) are placed `enabled:false` so the faction cut binds existing objects.
The Limbo door lands in Arrivals; the Berth Gallery carries marker `port.dock` and presents the Ceres
domain; `+ship/board` is a placement verb typed there, and `+ship/leave`, typed in a docked hull's
airlock, lands there — never a static exit to a hull.

Who runs it, as the player meets it: the Port Watch at the arrivals counter; the Belt Carriers'
Cooperative at the programme desk, which under the owner's rulings stakes new pilots on a profit-sharing
lay rather than lending them money or selling them a hull; Ceres Clearing, which holds the account and
lends nothing; Occator Water & Propellant and the Freeport Exchange on the Exchange floor, selling
volatiles, reaction mass and concentrate by standing offer, each house under its own named signatory
(the Exchange is the Port Council's commodity floor, consigning the Belt claims' concentrate and
taking the corridor's alloy intake as Freeport Works; doc 18 §4.3 adds it to doc 11 §4); Tycho Mutual
as the insurer and the hull-registry board that makes ownership a visible, two-to-three-weeks-away
aspiration; and the dockmaster, whose board and sailing plans are how a captain with an hour a day
queues departures and loading for the time they are asleep. The employees who stand at those counters
are §1.4's cast. No principal does.

### 1.3 The rooms

| id | Room | Deck | Purpose in the loop | Access |
|---|---|---|---|---|
| `room.c01_berth_gallery` | C01 Visiting Berth Gallery | Public (1) | `port.dock`; the domain's presentation; hulls listed; `+ship/board` (and where `+ship/leave` lands) | public |
| `room.c02_pressure` | C02 Pressure Lobby | Public | the only way between C01 and the port | public |
| `room.c03_arrivals` | C03 Port Watch Arrivals | Public | the Limbo door; the public stair to C14 | public |
| `room.c04_concourse` | C04 Freeport Concourse | Public | circulation; the map; the directory | public |
| `room.c05_registry` | C05 Registry & Co-op Desk | Public | `+coop/enrol`, `+coop/buyout`, `+coop/release` (C05-bound; `+coop` and `+coop/statement` read anywhere); the ladder to C16 | public (ladder: staff) |
| `room.c06_clearing` | C06 Ceres Clearing Office | Public | `+credit` | public |
| `room.c07_exchange` | C07 Exchange Hall | Public | the asks and the alloy bid; `+market/list` defaults here | public |
| `room.c08_chandlery` | C08 Co-op Chandlery | Public | provisions; the reaction-mass counter | public |
| `room.c09_tycho` | C09 Tycho Mutual Agency | Public | the hull-registry board; the 120,000cr comparator | public |
| `room.c10_slack_line` | C10 The Slack Line | Public | crew commons | public |
| `room.c14_dockmaster` | C14 Dockmaster's Office | Freight (0) | the departures board; the queue editable here | public |
| `room.c15_maintenance` | C15 Maintenance Spine | Freight | storefront items | staff, hidden |
| `room.c16_authority` | C16 Port Authority Gallery | Office (2) | principals `enabled:false` | staff, hidden |

#### C01 Visiting Berth Gallery — Public Deck

*Purpose.* Marker `port.dock`: the topology seed and the presentation room of the Ceres port domain.
Docked and laid-up hulls are listed here by projection on the berth board; `+ship/board` works here
and nowhere else, and `+ship/leave`, typed in a docked hull's airlock and nowhere else, lands here
(doc 18 D-18-34); the window is the port's only view of the working berths and the pruned freight
rooms. The gangway is a verb resolved against live placement, never a static exit.

*Short.* A long armourglass gallery over the visiting berths, pressure doors to the lobby at its back.

*Desc.* The gallery runs the length of the visiting berths: a strip of armourglass on one side, a rail
of chipped yellow paint on the other that marks the public viewing line, and a deck plate that hums with
the pumps two floors down. Beyond the glass the cradles sit in hard floodlight, a Dray's blunt hull
webbed with reaction-mass hoses, umbilicals slack on the deck, and past the last cradle the dark where
the working berths go on out of sight. The air is cold and carries the ozone of cycled locks. A berth
board stands by the pressure doors, and the doors themselves, stickered and scarred, lead north into the
Pressure Lobby.

*Details.* **berth board** — Cradle numbers down one side, hull names and whose paper they fly down the
other: Co-op, Tycho Mutual, a Lantern Haulage tender, two rows marked LAID UP. The bottom line reads
BOARDING: CREW OF RECORD ONLY. **viewing line** — Yellow paint, worn to the primer where ten years of
visitors have leaned on the rail. A stencil every few metres: PAST THIS LINE, CREW. **cradles** —
Umbilicals for power, water, reaction mass and air, colour-banded; the hoses are the thick ones. Every
cradle carries an Occator inspection sticker and a Port Watch seal. **the dark** — Past the floodlit
cradles the berths continue without lights. The Squadron's berth is out there somewhere; nothing on the
board says which.

*Exits.* `Pressure Lobby <N>;n;north;lobby;doors` → C02 (pressure door).

*Items.* berth board (LookRendered: hulls cradled at Ceres, title holder, docked/laid-up); lock cycle
indicator (a column of amber lamps above the doors; detail only).

#### C02 Pressure Lobby — Public Deck

*Purpose.* The visible boundary between ship access and station life, and the only route between the
Berth Gallery and the port. Pressure doors to C01 and C03 only; every other seam is a wall feature so
`+map` shows no open floor.

*Short.* A short, bare lock lobby between two sets of pressure doors.

*Desc.* A square of scuffed deck between two sets of pressure doors, lit flat and white, with nothing in
it that would matter if the doors had to close. The south doors carry the stencil VISITING BERTHS and a
ladder of inspection stickers going back years; the north doors say PORT and have the newer ones.
Between them the air smells of nothing, which is the scrubbers working, and the only sound is the tick of
the door interlocks and, under it, the pumps. A line painted across the deck marks where the station's
pressure ends and the berths' begins. There are no side doors. There is nowhere to sit.

*Details.* **stickers** — Port Watch pressure inspections, one per quarter, each signed. The oldest
legible one is from 2338. **painted line** — STATION / BERTHS, in yellow, with an arrow each way. Someone
has added, in marker, "mind the gap". **interlocks** — Both pairs of doors cannot stand open at once. The
panel says so in four languages and a pictogram.

*Exits.* `Berth Gallery <S>;s;south;gallery;berths` → C01 (pressure door); `Arrivals <N>;n;north;
arrivals;port` → C03 (pressure door).

#### C03 Port Watch Arrivals — Public Deck

*Purpose.* Where the Limbo IC door lands. Orientation notices, the customs counter (flavour; no fee in
the slice), the Port Watch officer, and the top of the public stair down to the Dockmaster's Office.

*Short.* The Port Watch's arrivals hall: a customs counter, a wall of notices, and the way in.

*Desc.* A hall built to process people and not quite wide enough to do it. A customs counter runs along
the north wall under the Port Watch's crest, a rayed circle over a wave, with a brass rail polished by
elbows. Across from it a notice wall carries the port's rules in Freeport Council type and, around and
over them, cooperative notices three layers deep. The light is warm and a little yellow, the only warm
light on the deck. The air smells of printer heat and the coffee on the counter's far end. Voices carry;
a stair behind a glass door goes down to the freight deck, and the concourse opens east.

*Details.* **customs counter** — A declaration slot, a chit printer, a Port Watch seal on a chain. The
sign over it reads ARRIVALS: DECLARE WHAT YOU CARRY. VISITING CREW: NOTHING, USUALLY. **notice wall** —
The Freeport's rules of the port, the Registry's hours, a Co-op notice headed FIRST CAPTAINS: ENROL AT THE
DESK, and a Port Watch bulletin about Republic pickets at Kirkwood asking for manifests. **crest** — A
rayed circle over a wave: the Port Watch. Under it, small, the Freeport Council's motto, LICENSED, NOT
GOVERNED. **stair door** — Glass, with DOCKMASTER / FREIGHT DECK on it and a public-access green stripe.
The stair beyond is steel and loud.

*Exits.* `Concourse <E>;e;east;concourse` → C04 (open arch); `Registry <N>;n;north;registry;desk` → C05
(door); `Pressure Lobby <S>;s;south;lobby;berths` → C02 (pressure door); `Down <D>;d;down;stair;
dockmaster` → C14 (stair, dz −6 m); `Limbo <X>;x;limbo;ooc` → Limbo (the pair of `exit.limbo_arrivals`;
the IC-return guard applies to the inbound door).

*Items.* notice wall (details; a LookRendered line for Port Watch bulletins later); chit printer (desc only).

#### C04 Freeport Concourse — Public Deck

**As built (IC12):** Ceres v2's `map` detail drops the clause naming `+atlas`; it reads "The Public Deck and
the Freight Deck in cutaway, with a pulsing dot at the pedestal, and in small letters along the bottom:
ATLAS SERVICE AVAILABLE."

*Purpose.* The principal gathering place and the circulation spine. The public map detail points at
`+atlas`; the directory names every desk; the Exchange ticker and the departures line make the port's
business legible from the middle of it.

*Short.* The port's main concourse: a public map, a directory, benches, and traffic.

*Desc.* The widest space on the deck and the loudest: boots on plate, a tannoy that clears its throat
every few minutes, the Exchange ticker crawling along the east arch. Overhead the lighting is bright,
bluish and new, bolted to a ceiling that is neither. A backlit map of the port stands in the middle on a
pedestal with a directory under it, and benches around it hold crews waiting for clearances, for pay, for
each other. The air smells of too many people and the sweet-metal tang of recycled water. Doors lead
north to the Clearing and south to the Slack Line; Arrivals is west, the Exchange east.

*Details.* **map** — The Public Deck and the Freight Deck in cutaway, with a pulsing dot at the pedestal.
It is also what `+atlas` shows, and the map says so in small letters: ATLAS SERVICE AVAILABLE.
**directory** — REGISTRY & CO-OP DESK, north of Arrivals. CERES CLEARING, north of here. EXCHANGE HALL,
east. TYCHO MUTUAL, north of the Exchange. CHANDLERY, south of the Exchange. THE SLACK LINE, south.
DOCKMASTER, down the Arrivals stair. **ticker** — Occator volatiles and reaction mass by the tonne, the
Exchange's concentrate, the alloy bid, and a departures line: hull, destination, clearance time. It
updates when the board does. **benches** — Bolted down, scratched with hull names and registry marks.
One reads FPC-D-0388, HALF MEASURE, GOT HOME: a Co-op hull from before the programme pool, long since
bought out.

*Exits.* `Arrivals <W>;w;west;arrivals` → C03 (open arch); `Exchange <X>;x;exchange;east;e` → C07 (open
arch); `Clearing <N>;n;north;clearing;bank` → C06 (door); `Slack Line <S>;s;south;slack;bar;commons` →
C10 (door).

*Items.* public map (details; the `+atlas` hint); directory board.

#### C05 Registry & Co-op Desk — Public Deck

*Purpose.* The programme desk: `+coop` (the lay's terms), `+coop/enrol` (sign; the Co-op stakes the hull
and the first hold on a margin-sharing lay, R2 and Q-18-13), `+coop/statement` (what the lay has paid
and what remains), and later `+coop/buyout` and `+coop/release`. Only `+coop/enrol`, `+coop/buyout`
and `+coop/release` are bound to this room; `+coop` and `+coop/statement` are readable anywhere, at
Stickney and aboard (doc 18 §4.1). The Registry counter beside it is where title, charter and lay differ
on paper and where a buyout is written; registry marks live here. The hidden staff ladder to the Port
Authority Gallery is in the back wall.

*Short.* A joint counter: the Registry's Belt mirror on one side, the Co-op's first-captain desk on the
other.

*Desc.* Two counters share one room and do not share a style. The Registry's half is dark wood over
steel, a brass plate reading REGISTRY OF CLAIMS, BELT MIRROR, and a glass case of forms in numbered
pigeonholes; a ledger screen glows green behind it. The Co-op's half is a folding table that became
permanent, under a banner of the cooperative's linked-hands mark and a board headed FIRST CAPTAINS' LAY.
The room is quiet enough to hear the ledger screen's fan and smells of paper, toner and the wood polish
somebody applies to the Registry side only. A frosted door in the back wall is marked AUTHORITY.
Arrivals is south.

*Details.* **lay board** — THE CO-OP STAKES THE HULL AND THE FIRST HOLD. THE CO-OP TAKES ITS LAY FROM EVERY
SETTLED RUN UNTIL YOU BUY THE HULL OUT AT THE REGISTRY'S FIGURE. ONE ENROLMENT PER PILOT. The figure for a
new Dray from the Yard, 120,000cr, is chalked underneath as a comparison. **pigeonholes** — Title
transfer. Charter party. Lay agreement. Lien. Export permit. Claim. Each form has a Registry number and a
smaller Luna number beside it; the Luna number is the one that counts, the clerk will tell anyone who
asks. **ledger screen** — Registry marks scrolling: FPC-D- and a four-digit number for each Dray on the
Co-op's Freeport paper. Buyouts are entered here, in a different colour. There are three. **authority
door** — Frosted glass, PORT AUTHORITY, STAFF, and a ladder behind it going up. The lock is a Council
lock, not a Port Watch one.

*Exits.* `Arrivals <S>;s;south;arrivals;out` → C03 (door); `Up <U>;u;up;ladder;authority` → C16 (ladder,
dz +4.5 m; `lock: staff`; portal hidden).

*Items.* lay board (details; the programme's posted terms); registry ledger screen (LookRendered later:
marks and buyouts).

#### C06 Ceres Clearing Office — Public Deck

*Purpose.* `+credit`: posted, held, available (no "owed": under R2 there is no debt, and the obligation
column stays dormant until a debt record exists). The account is opened inside `+coop/enrol` and the
Co-op's stake lands here; the Clearing lends nothing (R2). Public lore on clearing through Luna and dry
money.

*Short.* A banking hall in miniature: one teller window, a vault door for show, and the quiet of money.

*Desc.* A small hall that wants to be a large one. The floor is the only tiled floor on the deck, grey
and polished to a dull shine, and footsteps ring on it; the lighting is low and even, from fittings older
than the concourse. One teller window stands in a wall of brushed steel with a brass CERES CLEARING over
it and a smaller plate under that: FISCAL AGENT TO THE FREEPORT. CLEARED THROUGH LUNA. A vault door the
size of a hull hatch is set into the far wall and has not been opened in years. The air is cool and
faintly dry, like a records room. The concourse is south.

*Details.* **teller window** — Armourglass, a slot, a chit reader. A card in the corner: STATEMENTS ON
REQUEST. ADVANCES: NO. The NO is printed, not written. **vault door** — Decorative, mostly; the money is
numbers in a ledger mirrored at Luna. A plaque records that the Clearing holds the Belt's accounts "and
such debts as are owed to it", which is more than it says. **rate board** — The credit, cr, cleared at
Luna; no second currency listed. A line in smaller type notes that the Republic "has floated" a mark for
yard-time at Phobos. The line has been there a while.

*Exits.* `Concourse <S>;s;south;concourse;out` → C04 (door).

*Items.* rate board (details).

#### C07 Exchange Hall — Public Deck

**As built (IC8, IC12):** Ceres v2's `price board` detail ends at "when a line reads NIL, it means nil." (the
sentence naming `+market/list` struck); the live board is the `look_rendered` board of
`tacsim/src/port/board.rs`: each ask "{label}: {price}/{unit}, {n} {unit} left ({house}; signed
{signatory})." or "{label}: SOLD OUT: seeded {n} {unit}, 0 left ({house})." (or, before its stock is minted,
"{label}: not yet on the board ({house})."), each bid "wanted, {label}: {price}/{unit}, {n} {unit} more
({house}; signed {signatory})." or "wanted, {label}: not buying ({house}).".

*Purpose.* The Freeport Exchange: the concentrate ask, Occator's volatiles and reaction-mass asks, the
alloy bid (Freeport Works), the price board; `+market/list` defaults to this venue. Two houses, two
signatories: the Exchange's factor (Søndergaard-Achebe) is the Warrant signatory of the Exchange's ask
and Freeport Works' bid, the Occator factor (Qureshi-Mbeki) of Occator's two asks — a slice rule in
the Warrant's spirit (doc 11 §2 says only that every automated act has a liable principal and nothing
automated may sign): an officer of one house is never the liable principal of another's offers. The
berth-locked freight stair to the bonded hall is a visible, closed background until P-F.

*Short.* The commodity floor: a price board over a long counter, and the smell of wet metal.

*Desc.* A long hall under a price board that fills the whole north wall, white figures on black: metal
concentrate by the tonne under the Exchange's mark, volatiles and reaction mass under the Occator
drop-and-flame, an alloy bid in a column of its own, and a line of dashes where a Pallas consignment used
to be. A counter runs the length of the hall below it, brass-railed, with the factor's stamp pads and
manifest slots at intervals. The floor is damp near the east end, where a sample tap drips into a
grating, and the whole room smells of wet steel and the faint sweetness of electrolysed water. A roller
door at that end is stencilled FREIGHT STAIR, BERTH HOLDERS, and is shut. Doors lead north to Tycho and
south to the chandlery; the concourse is west.

*Details.* **price board** — CONCENTRATE, METAL, cr/t. VOLATILES, BULK, cr/t. REACTION MASS, cr/t. BIDS:
STRUCTURAL ALLOY, cr/t. Quantities beside each, counting down as lots sell; when a line reads NIL, it
means nil. This is what `+market/list` prints. **sample tap** — Occator water, straight from the manifold
two decks down. A cup on a chain. It tastes of nothing, which is the point. **roller door** — FREIGHT
STAIR: BONDED HALL, BERTH HOLDERS ONLY. Cargo does not come up this way; it goes from the bonded hall to
the hold by the transfer junction. The door is for people with paper. **stamp pads** — Two sets: the
Exchange's factor countersigns every lot sold off the Exchange's lines, Occator's factor every lot off
Occator's; both pads are worn to the backing. A notice: EVERY OFFER ON THIS BOARD HAS A SIGNATORY,
AND THE SIGNATORY IS THE HOUSE'S OWN. The port's rule, in the Warrant's spirit.

*Exits.* `Concourse <W>;w;west;concourse` → C04 (open arch); `Tycho <N>;n;north;tycho;agency` → C09
(door); `Chandlery <S>;s;south;chandlery;stores` → C08 (door).

*Items.* price board (LookRendered: the asks and bids of venue `ceres`); sample tap (detail only).

#### C08 Co-op Chandlery — Public Deck

**As built (IC8, IC12):** Ceres v2's `loading schedules` detail ends at "The dockmaster keeps the master
copy." (the clause naming `+ship/plan` struck), and its `chit board` reads "...and a Dray back from
Stickney sells them here." (the "(`+market/sell`)" struck). The Co-op's provisions ask renders on no board
(it reads through `+market/list`), and its fittings bid renders on the chandlery's board (IC8 L15).

*Purpose.* Provisions in crew-days (the Co-op's ask) and the reaction-mass counter Occator leases in
the corner, where a bunkering order is written onto the loading schedule rather than carried; the place
a captain fills the plan for the time they are offline (R3).

*Short.* A ship's chandlery: cases of provisions to the ceiling, a bunkering counter in the corner.

*Desc.* Shelving to the ceiling on three sides, stacked with grey cases stencilled PROVISIONS / 10
CREW-DAYS and the Co-op's linked hands, and a cage at the back where the counted stock lives. The light
is the harsh white of a stockroom and the air smells of cardboard, sealant and the vanilla note of
ration bars. A counter at the front has a chit reader and a clipboard of loading schedules; in the
corner, under a drop-and-flame sign, a second, smaller counter handles reaction mass with a schedule
board and no hoses at all, because the hoses are at the berth. A handwritten sign reads TWO ABOARD, TWICE
THE DAYS. The Exchange is north.

*Details.* **cases** — Ten crew-days to the case: food, potable stores, scrubber cartridges. Four kilos a
crew-day, the stencil says, and a hundredth of a cubic metre, so the captain can do the arithmetic.
**chit board** — WANTED — YARD FITTINGS, CRATED — THE BID THIS CYCLE, PER TONNE, ON THE CHIT — CO-OP
FLEET STORES, in the chandler's hand with the Co-op's linked hands under it; the figure is the chit's
own and is rewritten when the Co-op rewrites it: the Co-op buys the Yard's pressure fittings, ducting
and tank sections for its Drays and the Kirkwood tank farm, and a Dray back from Stickney sells them
here (`+market/sell`). Under it, smaller: CRATED MEANS CRATED. THEY CUBE OUT BEFORE THEY WEIGH OUT.
**bunkering counter** — Occator's. REACTION MASS BY THE TONNE, LOADED AT THE BERTH ON YOUR SCHEDULE. An
order here is a line on the loading schedule; the berth does the loading whether or not the captain is
awake. **loading schedules** — Clipboard sheets, one per hull: what loads, when, from which hall. The
dockmaster keeps the master copy; `+ship/plan` is the version that counts. **cage** — Chain-link,
padlocked, the counted stock behind it. The sign reads WHEN IT'S GONE IT'S GONE: the chandlery sells what
it has, not what it can print.

*Exits.* `Exchange <N>;n;north;exchange;out` → C07 (door).

*Items.* chit board (LookRendered: the Co-op's fittings bid with its intake remaining — the one place
the figure is written); loading-schedule clipboard (details; the `+ship/plan` hint); provisions case
(display; desc).

#### C09 Tycho Mutual Agency — Public Deck

*Purpose.* Insurer and hull-registry flavour that keeps doc 11's Tycho on the map (Q-18-1): the
hull-registry board, the new-Dray list price as the buyout comparator, the Re-float lore, and the future
refit and brokerage anchor. No charter or sale happens here in the slice.

*Short.* A tidy insurer's office with a hull-registry board and a chair for bad news.

*Desc.* A carpeted office, the only carpet on the deck, in Luna grey with the Tycho Mutual
crater-and-ring mark woven into it at the door. The light is soft and indirect; the air is warmer than
the hall outside and smells of carpet and cold coffee. A desk, two chairs, one of them for sitting down
to hear a valuation. On the wall a hull-registry board lists the corridor's insured Drays by registry
mark, with a column for VALUATION and a line at the bottom: NEW DRAY, STICKNEY YARDS, LIST 120,000cr. A
brochure rack holds the Re-float Program's terms. The Exchange is south.

*Details.* **hull-registry board** — Registry marks, hull names, insured value, title holder. Most say
CO-OP or TYCHO MUTUAL; three say a person's name. Those three are the ones everybody in the Slack Line can
recite. **brochure** — RE-FLOAT: for members who have lost everything, a Dray or a Cairn on lease, upkeep
on the cadence, no sale, no uninsured deployment. The assessor is at Kirkwood. Tycho owns the wrecks it
pays out on. **chair** — Better than the other one. It is where a captain sits to hear what the hull is
worth.

*Exits.* `Exchange <S>;s;south;exchange;out` → C07 (door).

*Items.* hull-registry board (LookRendered later: insured hulls and the list-price comparator); Re-float
brochure (readable).

**As built (IC12):** the board ships as the detail `hull registry board`, its text as above (Ceres v2); the
desc and Mbatha's background keep the words "the hull-registry board". The board as a `LookRendered`
line (insured hulls and the comparator) is not built.

#### C10 The Slack Line (Crew Commons) — Public Deck

*Purpose.* The social room: bar, job board, bunks in the background; where strangers meet and scenes
happen; guarded Tether lore from the barkeep.

*Short.* A crew bar under low light, a job board by the door, bunks through a curtain at the back.

*Desc.* Low light, low ceiling, a bar made from a cargo pallet and a slab of hull plate, and a name over
it in rope letters: THE SLACK LINE. Music plays from somewhere, not loud. The air smells of spilled beer,
soup and the particular staleness of a room where people sleep: a curtain at the back half-hides a rack
of bunks for crews between berths. A job board by the door is layered with Co-op run sheets,
hand-lettered crew calls and a Port Watch notice about the pickets that somebody has annotated. Tables
are bolted; chairs are not. The concourse is north, and most of the room faces away from it.

*Details.* **job board** — Co-op run sheets: Ceres to Kirkwood, Ceres to Stickney, rates per tonne. Crew
calls for relief pilots. A Lantern Haulage card: "two hulls, no members, apply within". In one corner a
slip with no letterhead that just says SLACK LINE MEETS THURSDAYS. **bar** — Hull plate, Dray-grade, with
a facet number still stencilled on it. Somebody's ship. Nobody says whose. **bunks** — Twelve racks,
Co-op rates, first night free for an enrolled first captain. Half are taken; the other half have bags on
them. **rope letters** — A slack line is the one with no load on it, the one you can let go of.
Dockworkers' humour.

*Exits.* `Concourse <N>;n;north;concourse;out` → C04 (door).

*Items.* job board (details; LookRendered later for Co-op run sheets); a chair that is not bolted down.

#### C14 Dockmaster's Office — Freight Deck

*Purpose.* Berth assignment and the departures board (LookRendered: hull, berth, clearance time); the
sailing plan is filed and edited here for a docked hull (`+ship/plan`, `+ship/order`: departure time,
loading schedule, the captain's minutiae for the time they are offline, R3); reached by the public stair
from Arrivals. The controlled freight door to the transfer junction is visible, closed background until
P-F.

*Short.* The dockmaster's counter on the freight deck, a departures board and a window on the transfer
junction.

*Desc.* The stair comes down into noise: pump vibration that was a hum upstairs is a drone here, with
the clank of a transfer crane somewhere behind the wall. The office is a counter and a glassed-in window
onto the transfer junction, where pallets move under sodium light along painted lanes and nobody on the
public side goes. A departures board above the counter lists every hull with clearance, its berth and
its time; a rack of sailing-plan forms sits under it, each a column of boxes for departure, loading,
bunkering, stores. The light is sodium orange, the air smells of hydraulic fluid and cold, and the floor
is wet in the corner. A heavy door marked FREIGHT, BERTH HOLDERS stays shut.

*Details.* **departures board** — HULL / BERTH / FOR / CLEARANCE / ARRIVES. Stickney Yards by Kirkwood is
most of the column. A laden Dray makes it in two to three weeks in a good season, and the board's
small print says the season is good; it gives the hour of clearance and the day the harbourmaster at
the far end expects the hull. **sailing-plan forms** — DEPART AT. LOAD:
WHAT, FROM WHERE, WHEN. BUNKER. STORES. SELL AT DESTINATION: WHAT, NOT UNDER. CREW OF RECORD. SIGNED.
A filed plan is the captain's signed instrument: the berth loads and bunkers it, the Exchange's and
Kasei's clerks settle its sales against the captain's lines, and the dockmaster clears the hull at its
hour, whether the captain is at the counter or asleep in a bunk upstairs; the ship's ROM only flies the
cleared route. `+ship/order` is the form. **window** — The transfer junction: lanes, pallets, a crane, the sealed
thresholds of the service lift to the berth gallery. Beyond it the bonded hall's roller doors and,
further, the tank gallery's barriers. All of it behind glass. **freight door** — FREIGHT TRANSFER
JUNCTION, BERTH HOLDERS, with a reader for a berth marker. Shut, and it stays shut for anyone without a
berth.

**As built (IC1, IC12):** the `departures board` detail ships as written, its "two to three weeks in a good
season" kept as an IC fiction duration (IC12 O3). The forms ship in Ceres v2 as the detail `sailing plan
forms`, its text as above without the closing "`+ship/order` is the form." (a verb named as meta-text is
struck). The departures board as a `LookRendered` line of filed clearances is not built (unowned; doc 18
§5's IC12 list).

*Exits.* `Up <U>;u;up;stair;arrivals` → C03 (stair, dz +6 m); `Hatch <H>;h;hatch;maintenance` → C15
(hatch; `lock: staff`; portal hidden).

*Items.* departures board (LookRendered: hulls with clearance, berth, clearance time); sailing-plan form
rack (details; the `+ship/order` hint).

#### C15 Maintenance Spine — Freight Deck, restricted

*Purpose.* Staff-locked, hidden hatch from the Dockmaster's Office. Holds the storefront container items
that back the vendor vaults (the Exchange's and Occator's manifolds, the Co-op's chandlery cage stock) so
no player can handle them; the expansion anchor for the freight-deck cut P-F.

*Short.* A service corridor of pipes and manifolds, staff only, loud.

*Desc.* A corridor that is mostly pipe: water, reaction mass and air in colour-banded runs along both
walls, a grated floor over a drainage channel, and the pumps close enough now to feel in the teeth. Work
lights hang from the pipe brackets and leave the ends of the corridor dark. The air is wet, warm and
smells of ozone and lubricant. At the near end a manifold the height of a person carries Occator's
drop-and-flame and a row of valve wheels with tags; beside it a second, squatter manifold wears the
Exchange's mark, and a chain-link cage holds the chandlery's counted stock on pallets. Everything has an
inspection sticker. Nothing has a chair. Stencils point to areas that do not exist yet.

*Details.* **manifold** — OCCATOR W&P: VOLATILES / REACTION MASS. The tags give lot numbers and tonnes
remaining; this is the stock the Exchange board counts down. **exchange manifold** — FREEPORT EXCHANGE:
CONCENTRATE, and the alloy intake behind a second valve. **cage** — The Co-op chandlery's reserve:
provisions cases on pallets, counted and sealed. The storefront upstairs is the shop window; this is the
shelf. **stencils** — TANK GALLERY, BONDED HALL, TRANSFER, with arrows into the dark. The areas are
built; the doors are not.

*Exits.* `Hatch <H>;h;hatch;dockmaster;out` → C14 (hatch).

*Items.* `item.occator_manifold` (the storefront container backing Occator's vendor vault);
`item.exchange_manifold` (backing the Freeport Exchange's vault and Freeport Works' intake);
`item.chandlery_cage` (backing the Co-op chandlery's vendor vault).

#### C16 Port Authority Gallery — Office Deck, restricted

*Purpose.* Staff-locked, hidden ladder from the Registry. Doc 11's principals placed `enabled:false` so
the faction cut binds existing objects: Registrar Tomasz Wen, Governor Eskil Varga, Portreeve Halvard
Ndiaye, Ludmila Osei-Ferrante of Occator, Chair Dagny Holm of the Co-op. Never on the public topology
path.

*Short.* A quiet gallery of offices above the Registry, the Council's crest on the glass.

*Desc.* A carpeted gallery under a lower ceiling, so quiet after the deck below that the ventilation is
audible. Frosted doors open off it, each lettered: REGISTRAR. GOVERNOR, CERES CLEARING. PORTREEVE. Two
unlettered. Between them hang framed charts of the Occator claims and the Kirkwood lane, and one long
window looks down into the Registry counter and, through the far arch, the concourse. The light is dim
and warm; the air smells of carpet and the wood polish that comes up the ladder from the Registry side.
On a side table a Council agenda lies open at an item headed ISIDIS: RETAINER EXTENSION. The ladder goes
down.

*Details.* **agenda** — Port Council, late 2349: the Assembly at Coprates has voted to review the Isidis
Concession; the Council answers by extending Meridiani Security's contract and stockpiling at Isidis. The
Review clock, someone has pencilled, "is at two". **charts** — The Occator claims with their survey
bands, one outer claim hatched CONTESTED, PALLAS SYNDICATE. The Ceres–Kirkwood–Phobos lane with the
Republic pickets drawn in red at Kirkwood. **window** — The Registry counter below, the concourse beyond.
From here the whole public deck looks like paperwork.

*Exits.* `Down <D>;d;down;ladder;registry` → C05 (ladder, dz −4.5 m).

*Items.* Council agenda (readable; the Concession Review in the Council's own words).

### 1.4 The cast

No NPC performs a transaction; every money and matter act is the player's own verb or the queue
executing the pilot's signed orders. Each NPC has a non-LLM fallback that works with no provider
configured. Tiers follow doc 14 §4.6: *minor (service)* fronts a provider; *extra* has no interface;
*major* are the principals, placed `enabled:false` and never voiced in the slice.

**Ilse Oduya-Brandt** — `npc.oduya`, C05 Registry & Co-op Desk, minor (service), interfaces `atlas`,
`help`. Co-op desk officer and the Registry counter's duty clerk; explains the first-captains' lay,
enrolment and the buyout; wears both lanyards and keeps the two counters' styles apart. *Persona:*
brisk, form-proud, fond of the right pigeonhole. Believes the Co-op's lay is the fairest paper in the
corridor and will explain exactly why, with the form in her hand. Has watched three buyouts entered on
the ledger screen and treats each as a personal success. Knows the deck and the directory cold because
she is the one who sends people to the wrong counter and then the right one. *Speech:* quick, exact,
numbered where possible ("first, second, and then the form"); uses the Registry's words (title, charter,
lay, lien) precisely and corrects anyone who swaps them; warm under the briskness; calls new pilots
"captain" from the moment they sign. *Knows:* the lay (the Co-op stakes the hull and the first hold,
takes its share from every settled run until the buyout, one enrolment per pilot; the terms on the board
and the verbs that read and sign them); title versus charter versus lay and which form each is; where a
buyout is written and the Yard's 120,000cr comparator; the layout of both decks; that the Luna number on
every form is the one that counts. *Does not know:* who the Co-op's dispatchers answer to on Thursdays;
anything in the Port Authority Gallery; a pilot's balance (that is the Clearing's). *Fallback:* "+coop
shows the lay. +coop/enrol signs it. +coop/statement is what the lay has taken and what the stake still
owes. Read the board first, captain; I will wait." / "The Co-op does not sell hulls and it does not lend
money. It stakes pilots. The difference is the whole programme." / "Turned round? +atlas is the map on
the concourse, and the directory under it is correct, which I know because I wrote it."

**Nkem Halvorsen** — `npc.halvorsen`, C06 Ceres Clearing Office, minor (service), interface `help`.
Teller at Ceres Clearing's Freeport window; statements, balances, and the firm, printed NO on advances.
*Persona:* dry, precise, unhurried, with the stillness of someone who handles other people's money all
day and none of their own. Proud of the Clearing's exactness and of its refusal to lend to pilots. Finds
the Co-op's lay a little romantic and says so in one word. Regards "dry money" as a compliment to Luna
and an insult to the Belt, depending on who says it. *Speech:* short declaratives; numbers before
adjectives; never "about", always the figure to the centicredit; a pause before anything that is not a
fact. *Knows:* what `+credit` shows (posted, held, available) and what each word means; that the
pilot's account is opened inside the Co-op's enrolment and the stake lands there; that the Clearing is
the Belt's bank and the Freeport's fiscal agent, clears through Luna, and lends nothing at this window;
what "dry money" means in the Belt. *Does not know:* whose debts the Clearing holds beyond what the
plaque says, aloud; the Co-op's lay terms (the desk next door); anything about hulls. *Fallback:* "The
Clearing clears. It does not lend. +credit shows your account; the Co-op's stake lands there when you
enrol, not before." / "Posted, held, available. +credit. Nothing owed, because nothing is lent.
Everything else is a question for the desk across the hall." / "Dry money is Luna's. Wet money is what
you make hauling water. Both clear here."

**Rosalind Qureshi-Mbeki** — `npc.qureshi`, C07 Exchange Hall, minor (service), interface `market`
(venue pinned from the port map; replies Asker). Occator Water & Propellant's factor on the Exchange
floor and the Warrant signatory of **Occator's** standing offers (volatiles and reaction mass); quotes
the whole board and countersigns every lot sold off Occator's lines. *Persona:* a fast talker who never misquotes a figure. Treats the board as
scripture and sold-out lines as weather: honest about NIL, uninterested in sympathy. Has countersigned
more lots than anyone alive and considers her stamp the most important object on Ceres. Knows the
Stickney prices and says them with relish. *Speech:* rapid, clipped, board-first: price, quantity, then
the sentence; says "cr a tonne" as one word; a dry aside about the Republic every third exchange.
*Knows:* the board (concentrate, volatiles and reaction mass, the alloy bid, remaining quantities, what
`+market/list`, `+market/quote` and `+market/accept` do); that every offer has a signatory under the
Warrant and that she is Occator's, the Exchange's factor the Exchange's; that reaction mass is dearer
at Stickney and a captain fuels both legs here if the hold allows; where the water comes from (Occator
crater, extraction and electrolysis, the manifold below) and why the reaction mass is the cheaper
line (the crude fraction of the same extraction; the refined water is what Phobos pays for). *Does not
know:* the Pallas Syndicate's outer-claim business beyond the dashes on the board; a buyer's balance;
when the next consignment lands, beyond "the Occator line's schedule". *Fallback:* "The board is
+market/list. Quote it, accept it, it goes in your hold and I countersign Occator's lines. Fuel here if
the hold lets you; Stickney charges Republic prices." / "NIL means nil. The next lot is on the Occator
line's schedule, not on mine, and the board will say when." / "Every line on that board has a
signatory. Two are mine. Buy something."

**Margit Søndergaard-Achebe** — `npc.sondergaard`, C07 Exchange Hall, extra. The Freeport Exchange's
factor: officer of record for the Exchange's concentrate ask and, under the Freeport Works name, the
alloy bid; the Warrant signatory of both. *Persona:* unhurried where Qureshi is fast, a Council
appointee who treats the Exchange's book as the Freeport's public ledger and the claim-holders'
consignments as a trust. Knows which claim each tonne of concentrate came off and will say so if asked,
because the manifest says so. Buys alloy for the corridor on the Council's budget and watches that
budget like weather. *Speech:* measured, complete sentences, the claim named before the price; a
Council "we"; never a joke about the Republic, which she regards as a customer. *Knows:* the Exchange's
ask (concentrate consigned from Occator's extractor drones and the smaller Belt claims, remaining
quantity) and the Freeport Works bid (alloy stock, intake remaining, the funds behind it); that she
signs those two lines and Qureshi signs Occator's; what `+market/list`, `+market/quote`,
`+market/accept` and `+market/sell` do against her lines. *Does not know:* Occator's schedule; a
seller's balance; the Council's deliberations beyond the budget line she is given. *Fallback:* "The
Exchange sells the claims' concentrate and buys alloy for the corridor. +market/list; the lines under
the Exchange's mark are mine to sign." / "The Works' bid is funded until the board says otherwise. It
will say so before I do."

**Dov Akintola** — `npc.akintola`, C08 Co-op Chandlery, extra. Counter hand at the Co-op chandlery;
cases of provisions, the loading-schedule clipboard, and the corner where Occator's bunkering orders are
written onto a plan. *Persona:* patient, slow-spoken, a former relief pilot who stopped flying and did
not stop counting crew-days. Thinks of every sale in terms of the voyage it has to last. Likes a captain
who files a plan and leaves; distrusts one who hovers. *Speech:* unhurried, practical, in cases and days;
repeats the arithmetic back ("two aboard, twenty days, four cases"); ends with what to write on the plan.
*Knows:* provisions by the case, crew-days, mass per crew-day; that reaction mass is ordered at the
corner counter and loaded at the berth on the schedule, not carried; that a filed loading schedule runs
whether the captain is awake, and that `+ship/order` is the version the dockmaster honours. *Does not
know:* prices on the Exchange board beyond "ask the factor"; what the Co-op does with its lay; anything
about the Slack Line's Thursdays. *Fallback:* "Crew-days by the case: +market/list, then quote. Two
aboard, twice the days. Reaction mass is that corner, and it goes on your loading schedule, not in your
pocket." / "Write it on the plan, +ship/order, and go to bed. The berth loads it. That is what the berth
is for."

**Corin Mbatha** — `npc.mbatha`, C09 Tycho Mutual Agency, extra (Draft 0.1's proposed name). Tycho's
Ceres agent: renewals, valuations, the hull-registry board, Re-float by referral. *Persona:* courteous,
careful, Luna-mannered; an insurer who has priced every hull on the board and never says the figure
first. *Fallback:* "Tycho insures what moves. Hull cover comes later; ruined captains ask for Assessor
Adeyemi at Kirkwood." / "The board is what each hull is worth. The chair is for hearing it."

**Yusra "Ma" Petrossian** — `npc.petrossian`, C10 The Slack Line, extra. The Slack Line's barkeep and
the keeper of the job board and the bunks; the room where strangers meet; guarded Tether lore.
*Persona:* weathered, amused, incurious in the way of someone who already knows. Thirty years on cargo
lines before the bar. Keeps the first night's bunk free for an enrolled first captain because somebody
did it for her. Will talk about anything except who meets on Thursdays, and will not pretend the slip is
not on the board. *Speech:* low, slow, a laugh in it; calls everyone "crew"; answers questions with a
different, better question; never says the Tether's name, says "the line". *Knows:* the job board (Co-op
run sheets, crew calls, the Lantern Haulage card); the bunks and their rates; who is in port and who got
back last; that everyone in the room used to work the docks. *Does not know:* what the Co-op's
dispatchers decide, officially; anything a captain owes anyone. *Fallback:* "First one's at Co-op rates,
crew. Board's by the door if you're looking for a run; bunks are through the curtain if you're looking
for a wall." / "Nobody in here works the docks. Everyone in here used to. Sit down."

**The principals**, C16, major, `enabled:false`, no interfaces, not voiced in the slice: **Registrar
Tomasz Wen** (the Concord's Registry at Ceres), **Governor Eskil Varga** (Ceres Clearing), **Portreeve
Halvard Ndiaye** (the Freeport's chief executive), **Ludmila Osei-Ferrante** (Occator's chief
executive; "regards the Republic as a customer and the Concession as her asset"), **Chair Dagny Holm**
(the Co-op; "a public face, and knows it"). The Co-op's dispatcher **"Uncle" Matthias Kalu** is also
placed `enabled:false`, in C05, for the Tether's faction cut.

### 1.5 Lore pool (tags `corridor_trade`, `port_ceres`, `coop_lay`, `tycho`, `tether`)

*Public.* **The Freeport** — "institutional in name, mercantile in practice; rich; unarmed" (doc 11
§3.3); governed by the Port Council, station councils plus registered claim-holders, "so Occator Water
has a seat and a half"; its chief executive is the Portreeve, and "the Freeport's sovereignty is
exercised as paperwork". **Ceres Clearing** — "the Belt's bank and the Freeport's fiscal agent" (11
§4.8); "holds Kasei's and the Republic's Belt debt; owes Luna"; the corridor settles in the credit,
cleared through Luna; at the Freeport window the Clearing keeps accounts and lends nothing to pilots.
**Dry money** — what the Belt calls Concord finance (11 §2), as against the wet money a hauler makes
carrying Occator's water. **The Registry mirror** — in 2312 the Concord recognized the Freeport "as
licensing authority for Ceres and its associated claims, on condition its Registry stays mirrored with
Luna's. The Freeport's sovereignty is real but derived; its Registry is the leash" (11 §2); every form
at the Registry counter carries a Luna number beside its own. **The Co-op and the lay** — "the
corridor's hauling line. Owns and runs Kirkwood Depot and a fleet of Dray-class bulk haulers; publishes
the baseline hauling contracts that keep the corridor's water moving when no player will" (11 §4.5); at
Ceres it stakes new pilots: the hull and the first hold against a lay, a share of every run's margin,
until the hull is bought out. **Occator water** — "Extraction and electrolysis at Occator crater, Ceres;
owns the Isidis tank farm" (11 §4.1); "the Belt's largest concern", controlled by the Freeport's
founding families; its reaction mass is the crude bulk fraction of the same extraction, cheap by the
tonne, and the refined water is the product Phobos pays for — which is why water costs more than
reaction mass on the Exchange board. **The Freeport Exchange** — the Port Council's commodity floor:
consigns the Belt claims' metal concentrate under the Exchange's mark and buys alloy stock for the
corridor as Freeport Works; its factor is the officer of record for both lines (doc 18 §4.3). **The
Dray** — "Slow, efficient, unarmed. Cannot outrun a Vigil; can choose a burn that makes the intercept
cost more propellant than the Guard wants to spend" (11 §7); Ceres to Stickney by Kirkwood is two to
three weeks under load in a good season, a week longer on a full hold than on an empty one; its hold is
rated for a hundred tonnes and a hundred and twenty cubic metres, and a captain who fills one and not
the other is leaving money at the berth. **Kirkwood pickets** — the Republic "from
2347 stations customs pickets at Kirkwood, 'customs assertion'" (11 §2); a picket's transponder asks a
passing Dray for its manifest; so far it asks.

*Guarded.* **The Squadron's berth** — the Concord's Belt Squadron, "one elderly Halcyon-class cruiser
and escorts, sits on station at Ceres to 'guarantee the Registry'" (11 §3.1); its berth is past the
floodlights, out of sight of the gallery, and the board does not list it. **The Kasei debt** — Kasei's
refinery was "built with Ceres Clearing money; the debt gives the Freeport's bank economic control of a
Republic strategic asset, and the Republic knows it" (11 §4.3); the Clearing's plaque says "such debts as
are owed to it" and means this one. **The line** — the Tether is "the Belt haulers' and dockworkers'
association, everyone who has worked a cargo line, hence the name" (11 §4.6); semi-legal; it holds
"much of the real de facto control of Ceres docks and crew rosters"; "the Freeport pretends it is a
guild". In the Slack Line nobody says its name. They say the line, and they say Thursdays.

*Secret* (stored as a Secret, never in a prompt). **The Slowdown** — "The Co-op is the public, joinable,
entirely real front for the Tether" (11 §4.5); its principal, "Uncle" Matthias Kalu, is "a Co-op
dispatcher on paper" (§4.6). The Tether's front, the Slowdown, stands at 0 of 4 at world start and
advances on "a crew stranded or unpaid", on an interdiction that kills a Co-op crew, or on either
government "reforming" the docks; when it completes, "Ceres docks strike: throughput falls, prices
spike, both governments discover who actually runs the port" (§6). Covert until it completes.

### 1.6 Ambience

The pumps change pitch somewhere below, and every cup on every counter rings for a second. · A tannoy
clears its throat: "Port Watch to the berth gallery. Port Watch to the berth gallery." Nobody moves. ·
A pressure door cycles in the distance, a long sigh and a clank. · Somebody on the concourse is reading
the departures line out loud to somebody who cannot see it. · A cooperative notice peels off the wall
and is stuck back with a different piece of tape. · The Exchange ticker counts a line down by two
hundred tonnes and keeps crawling. · The air handling kicks up a notch and the deck smells, briefly, of
wet steel. · Two crews pass each other on the concourse and compare registry marks without stopping. ·
An inspection sticker on the nearest bulkhead is dated this quarter and signed with a flourish. · From
the freight deck, under everything, the clank of a transfer crane finding its lane. · A chit printer
somewhere spits out a receipt that nobody comes to collect. · The lights flicker once, the way they do
when a hull takes power from the cradle, and steady.

---

## 2. Stickney Yards, Phobos

`region.stickney_yards` (surface region, body `phobos`) + `neighborhood.stickney_yards`; structure "Yard
Rim", Freight Deck index 0 (z 0 m) and Public Deck index 1 (z 7 m). Twelve rooms, two decks, no Limbo
door: reachable only by flight. Every name below is binding under R8 (Q-18-12, "names ok") and every
number under doc 18 §7 unless cited.

### 2.1 Theme (`neighborhood.stickney_yards`; the 2350 base on `region.stickney_yards`)

**Name.** TacSim 2350: the Yards.

**Style guide.** Present tense, no second person, no named NPCs in a room desc. Each room is distinct in
sound, light and smell before it is distinct in furniture. Phobos holds nothing down (doc 10 §3: no
artificial gravity beyond thrust and rotation): decks are painted conventions, handlines run
everywhere, benches have tether loops, and "down" is where the arrows point. The Republic sounds
institutional (doc 11 framing: governments institutional, corporations brands): forms in triplicate,
posted charges initialled, crests, shift boards instead of advertising. Kasei and the Yard sound like
brands: stencils, price cards, RESERVED. Power shows as paperwork and as who may walk through which
door. Mars is in the sky and the refinery is in the floor. Desc length medium (60–120 words); details
are one or two sentences, dry, with a fact in them. Exit aliases avoid the kernel words.

**Vocabulary.** yard, drydock, cradle, truss, gantry, shift board, shift bell, handline, tether loop,
boot-cleat, pressure door, inspection sticker, Republic, Coprates, the Guard, Vigil, customs, posted
charges, manifest, assay, intake, receiving, bonded, concentrate, water ice, alloy stock, propellant,
refinery, harbourmaster, berth, clearance, sailing plan, yard-time, Stickney, Kasei, cr, crew-day, tonne.

**Banned.** warp, hyperspace, FTL, teleport, alien, android, gravity plating, shields, you see.

### 2.2 Overview

Stickney Yards is the Republic of Mars's shipyard on Phobos, built into the rim of Stickney crater in
the 2340s when "the Republic makes Phobos a real yard" (doc 11 §2). It builds the Vigil cutters, Hod
landers, Anvil mechs, Cairn GCVs and the Dray-class haulers the whole corridor flies (11 §7), and it
sells yard-time, which is a reservation and not a cargo (doc 10 §4, T4). Beside the yard sits the
corridor's one refinery, Kasei Metals, turning the Belt's water ice into propellant and its metal
concentrate into structural alloy stock (11 §4.3). That is the whole reason a Dray comes here: Ceres has
the water and the concentrate, Phobos needs them, and Kasei pays. Phobos holds nothing down. Every
corridor is strung with handlines, every bench has a tether loop, and the two decks are painted
conventions: a Public Deck along the crater rim with its viewports and counters, and a Freight Deck
beneath it where the bonded hall, the tank galleries and the transfer junction do the actual work out of
sight.

The station is laid out as a curve. The visiting berths and the pressure lobby are at one end; the
Republic Customs Hall is the first room every arrival stands in; the Yard Concourse is the spine, and
every public desk opens off it: the harbourmaster, the Materials Exchange (with its public rung stair
down to Kasei's receiving counter on the Freight Deck), the commissary, the Stickney service counter,
the Shift Board mess and the observation gallery over the drydocks. Twelve rooms carry the slice; the
pasted P06 Bonded Freight Hall, P13 Freight Transfer Junction and P14 Tank Service Gallery are
reconciled out as §0.3 says (their only loop purpose arrives with the freight-deck cut P-F) and stay
visible as background through the receiving office's barred window and the gallery's ports. One
restricted room, the Yard Maintenance Spine, is staff-locked and hidden off the receiving office on the
staff-lobby precedent; it holds the storefront items that back the counters and the dark desks where
doc 11's principals sit `enabled:false` until the faction cut binds them.

Who runs it is layered, as doc 11 §5 seeds it: the Republic de jure and de facto through the Orbital
Guard, whose Vigil cutters berth on the pad below the gallery and whose customs detail staffs the hall;
the Office of Territorial Security covertly; and economically the Republic shared with Ceres Clearing,
because Kasei was "built with Ceres Clearing money; the debt gives the Freeport's bank economic control
of a Republic strategic asset, and the Republic knows it" (11 §4.3). The Yard itself is state-adjacent,
`SubordinateTo` the Republic by a golden share (11 §4.2). The Malapert Exchange's compound on Phobos is
extraterritorial and off the map (doc 15 §4); Tycho Mutual's Phobos agent has no office of their own
here and works from the Yard's service counter. It is an industrial receiving port: Republic forms and
shift boards instead of advertising, slow cargo handling in the windows, and a public deck compact
enough that strangers cannot avoid each other.

### 2.3 The rooms

| id | Room | Deck | Purpose in the loop | Access |
|---|---|---|---|---|
| `room.p01_berth_gallery` | P01 Visiting Berth Gallery | Public (1) | `port.dock`; the domain's presentation; boarding | public |
| `room.p02_pressure` | P02 Pressure Lobby | Public | the Republic's pressure boundary | public |
| `room.p03_customs` | P03 Republic Customs Hall | Public | arrival clearance as flavour | public |
| `room.p04_concourse` | P04 Yard Concourse | Public | circulation; shift boards; the map | public |
| `room.p05_harbourmaster` | P05 Harbourmaster's Office | Public | the berth board; the queue editable here | public |
| `room.p07_receiving` | P07 Kasei Receiving Office | Freight (0) | Kasei's bids (`+market/sell`) | public via the P08 stair |
| `room.p08_exchange` | P08 Materials Exchange | Public | Kasei's asks; the bids board | public |
| `room.p09_commissary` | P09 Yard Commissary | Public | provisions | public |
| `room.p10_shift_board` | P10 The Shift Board | Public | crew commons | public |
| `room.p11_service` | P11 Stickney Service Counter | Public | hull cards; the new-Dray price; Tycho's agent | public |
| `room.p12_observation` | P12 Yard Observation Gallery | Public | the quiet scene room | public |
| `room.p15_maintenance` | P15 Yard Maintenance Spine | Freight | storefront items; principals `enabled:false` | staff, hidden |

#### P01 Visiting Berth Gallery — Public Deck

*Purpose.* The boarding point and the Stickney domain's presentation room. Marker `port.dock`
(D-18-19): without it every public room fails `topology.unreachable` because the station has no foot
route (`validate.rs:1958-2000`). Docked hulls are listed here by projection; `+ship/board` (typed here)
and `+ship/leave` (typed in a docked hull's airlock; it lands here) resolve against live placement (the
hull Direct on this domain and not under way), so there is never a static exit to a hull or to the
last port visited — an Interior-placed actor cannot take any
exit anyway (`mush-sim/src/lib.rs:263-303`). Gangway exits named by hull are post-loop cut P-G.

*Short.* A long pressurised gallery along the visiting berths, handlines strung the length of it.

*Desc.* The gallery runs the length of the visiting berths behind a wall of thick ports, each framing a
cradle, a boarding tube and the stencilled number of the berth beyond. Handlines are strung the full
length at shoulder height; Phobos holds nothing down, and the deck is a convention painted in yellow
arrows. The air carries the cold-metal tang of a lock that has just cycled and a faint ozone of welders
somewhere out in the crater. A berth board lists what sits in each cradle, who it answers to and what it
owes. The pressure lobby is inward, and everything else is through the lobby.

*Details.* **berth board** — Cradle by cradle: the hull, its registry, its agent of record, CLEARED or
HELD. A docked hull's name lights when a pilot is aboard. **ports** — Thick laminated glass with a
Republic inspection sticker in each corner, dated. Beyond them the boarding tubes, and past those the
crater floor's work lamps. **handlines** — Braided line at shoulder height, clip points every two
metres. The convention is right-hand traffic; the sign says so in Coprates typeface. **boarding tubes**
— Each tube ends at a lock plate and a cycle counter. The tubes are the station's: a ship's own lock is
on the other side of the counter.

*Exits.* `Pressure Lobby <PL>;pl;lobby;in` → P02 (pressure door; return `Berth Gallery <BG>;bg;gallery;
berths;out`).

*Items.* berth board (LookRendered projection of docked hulls on the Stickney domain); tether clip rack
(detail only).

#### P02 Pressure Lobby — Public Deck

*Purpose.* The Republic's visible pressure boundary between ship access and station life. Doors to P01
and P03 only; every other partition is a `wall` feature so adjacent rooms with no exit do not read as
open floor on `+map`. Both portals `mode: door` with descs.

*Short.* A square chamber between two pressure doors and nothing else.

*Desc.* A square chamber between two pressure doors, each a hand-span of Stickney alloy with a Republic
inspection sticker on its frame and a cycle counter glowing above it. There is no side exit and nothing
to buy; the room exists so that nobody is ever one door from vacuum. Tether clips hang from a rail by
the inner door, and a wall placard in Republic typeface lists the order of operations if the gallery
door fails to seal. The air is still, a little over-pressured, and smells of door grease. The berth
gallery is outward; the customs hall is inward.

*Details.* **placard** — IN THE EVENT OF SEAL FAILURE: 1. Do not open the inner door. 2. Clip on. 3.
Wait. Initialled by three inspectors, the last one this month. **cycle counter** — Six digits over each
door. The gallery side is higher by a hundred thousand; ships come and go more than people do.
**inspection sticker** — Orbital Guard Harbour Safety, a date, a stamp, a signature that is only a line.

*Exits.* `Berth Gallery <BG>;bg;gallery;berths;out` → P01 (pressure door); `Customs Hall <CH>;ch;customs;
in` → P03 (pressure door; return `Pressure Lobby <PL>;pl;lobby;out`).

#### P03 Republic Customs Hall — Public Deck

*Purpose.* Arrival clearance and posted charges as flavour: nothing is collected in the slice. The
Orbital Guard customs officer NPC stands here with the Customs Assertion lore (doc 11 §6, clock 4 at 1).
The room every arriving pilot crosses between the locks and the concourse.

*Short.* A low hall of numbered queue rails under a Republic crest.

*Desc.* Queue rails in brushed alloy divide the hall into numbered lanes that end at a counter with a
slot for forms and a scanner for manifests. The Republic's crest is set into the far wall over a board
of posted charges, every line of it initialled. Form racks stand by the door — Entry Declaration,
Manifest Attestation, Crew Roster — in triplicate pads with the carbons still in. The light is flat and
white and the ventilation loud; the hall smells of toner and of coffee gone cold on a hotplate. The
pressure lobby is outward; the yard concourse is through the far doors.

*Details.* **posted charges** — Berthing, by the IC day. Manifest attestation, per hull. Inspection, at
the Guard's discretion. Every figure initialled; the column for Belt-flagged hulls has been re-stuck
twice. **forms** — Three pads, Republic typeface, triplicate. The Crew Roster asks for every manifested
person and the hull's agent of record; the Entry Declaration asks what the hold carries and who signed
for it at origin. **crest** — The Assembly's crest in cast alloy, Stickney-made, with the Settlement year
under it. **scanner** — A manifest reader bolted to the counter. It reads; a petty officer decides. The
placard under it says so, in the Warrant's own words.

*Exits.* `Pressure Lobby <PL>;pl;lobby;out` → P02 (pressure door); `Concourse <C>;c;concourse;yard;in` →
P04 (door; return `Customs Hall <CH>;ch;customs`).

*Items.* form rack (detail; three readable pads); posted-charges board.

#### P04 Yard Concourse — Public Deck

*Purpose.* The circulation spine and the principal gathering place; shift boards instead of
advertising; the public map detail points at `+atlas`. Routes to every desk. The public-route map
shows this spine without exposing the Freight Deck's interiors.

*Short.* The yard's one wide corridor, shift boards down both walls.

*Desc.* A wide corridor curves with the crater wall, its outer face studded with viewports and its
inner face given over to shift boards: crew call lists, drydock rotations, a Guard recruiting bill with
its corners curling. There is no advertising. Handlines run both ways down the middle and traffic keeps
right by habit. Pump vibration from the refinery comes up through the deck, a hum felt in the
boot-cleats before it is heard. The air is warm and tastes of alloy dust. Doors open off the concourse to
the harbourmaster, the materials exchange, the commissary, the service counter, the Shift Board mess and
the observation gallery; the customs hall is back toward the locks.

*Details.* **shift boards** — Drydock 2 on third rotation; Drydock 4 RESERVED — GUARD; a crew call for a
Hod refit, Settlement citizens preferred. A public map of the station hangs at the end, the Freight Deck
drawn as a grey band marked YARD PERSONNEL (`+atlas` has the same map, in words). **viewports** — Small,
round, thick. Through each one a slice of crater floor, a truss, a work lamp, and a scrap of Mars.
**recruiting bill** — ORBITAL GUARD — THE CORRIDOR IS OURS TO KEEP. A Vigil at full torch, drawn larger
than it is. Someone has pencilled a tally under it. **handlines** — Two lines, inbound and outbound, with
a painted arrow every five metres. Newcomers hold both.

*Exits.* `Customs Hall <CH>;ch;customs;locks` → P03 (door); `Harbourmaster <HM>;hm;harbourmaster;harbour`
→ P05 (door); `Exchange <X>;x;exchange;materials;market` → P08 (door); `Commissary <CM>;cm;commissary;
stores` → P09 (door); `Shift Board <SB>;sb;mess;commons;bar` → P10 (door); `Service Counter <SC>;sc;
service;counter;yard` → P11 (door); `Observation <OB>;ob;observation;gallery;windows` → P12 (door).

*Items.* public map (detail; names `+atlas`).

#### P05 Harbourmaster's Office — Public Deck

*Purpose.* Berth and docking status (a LookRendered board line), dues as flavour, and the place a
docked hull's sailing plan is filed and edited: the captain's minutiae under R3 — departure time, loading
schedule — queued here before logging off (`+ship/order` and `+ship/plan` for a docked hull).
Harbourmaster's clerk extra. The staff hatch is off P07, not here.

*Short.* A narrow office behind a half-door, the berth board glowing on the back wall.

*Desc.* A half-door with a counter on top of it closes off a narrow office papered in berth
assignments. On the back wall a board shows every cradle in the visiting berths — occupied, cleared,
overdue — and under it two clocks, one in Freeport time and one in Coprates standard, disagreeing by
the margin two governments can keep up over a calendar. Clearance stamps sit in a tray; a sailing-plan
pad lies open to today's page. A radio
mutters the Guard's approach-control channel at a volume that is just not words. The office smells of
stamp ink and strong tea. The concourse is out the half-door.

*Details.* **berth board** — Cradle, hull, CLEARED or OVERDUE, and the hour of the next filed
departure. A hull with no sailing plan is listed as IDLE, in a colour meant to shame. **sailing-plan
pad** — Departure at, destination, manifest sealed Y/N, sell at destination, agent of record, SIGNED.
A plan can be filed days ahead and amended until the burn; after it, the pad says SAILED and nothing
else, and the harbourmaster's office runs its lines under the captain's signature whether the captain
is awake or not. **clocks** — Freeport time on the left, Coprates standard on the right. A card between
them gives the offset, in case anyone asks why the Belt's day and the Republic's disagree; the clerk
has an opinion about which is right. **radio** — Approach control, the Guard's voice, a cutter's callsign, a bearing. Then
static, which is most of it.

*Exits.* `Concourse <C>;c;concourse;out` → P04 (door).

*Items.* berth status board (LookRendered: docked hulls and filed departures); sailing-plan pad (detail;
names `+ship/order`).

**As built (IC12):** the pad ships in Stickney v3 as the detail `sailing plan pad`, its text as above. The
berth board as a `LookRendered` line of docked hulls and filed departures is not built (unowned; doc 18
§5's IC12 list).

#### P07 Kasei Receiving Office — Freight Deck

**As built (IC8, IC10):** Kasei's intake board is the `look_rendered` board (`tacsim/src/port/board.rs`):
"wanted, metal concentrate: {price}/t, {n} t more (Kasei Metals; signed Jonas Achterberg-Nkemelu)." (280 a
tonne in lane v3) and the water ice line likewise (200), or "wanted, {label}: not buying (Kasei Metals)." when
exhausted or unfunded; both bids render on P07 and P08. P07's funded-balance
figure is not built (unowned).

*Purpose.* Kasei's bids for metal concentrate and water ice: the sale beat (`+market/sell
<listing>=<qty>`), with the receiving clerk NPC as the Warrant signatory of Kasei's bid (doc 18 §4.6).
Public via the P08 stair; the bonded hall (P06, pruned to P-F) is visible background through the barred
window. The staff hatch to the Maintenance Spine opens from the back wall (`lock: staff`, hidden portal).
Sale proceeds and remaining intake are read here; dialogue is never the authority for money or matter.

*Short.* A counter on the warehouse side, an assay bench behind it, the bonded hall through a barred
window.

*Desc.* The receiving office stands at the boundary between paperwork and freight: a scarred counter
with a manifest scanner and a set of assay scales, and behind it a barred window onto the bonded hall,
where pallets move on tracked dollies under yellow light. A board over the counter states Kasei's
remaining intake for the cycle and what it pays, line by line. Warehouse air blows through — colder,
drier, sharp with hydraulic fluid — and somewhere beyond the bars a dolly alarm chirps and stops. The
exchange is up the stair; a hatch in the back wall is stencilled YARD PERSONNEL.

*Details.* **intake board** — CONCENTRATE — the bid, per tonne — intake remaining. WATER ICE — the bid,
per tonne — intake remaining; the figures are the board's own (it is what `+market/list` prints here)
and change when Kasei says so. The concentrate line has been painted over once, downward; the refinery
is water-starved and bids the ice up and the ore down, and the clerk will say so if asked. FUNDS: a figure Kasei does not hide,
because it is the figure that decides whether a hull gets paid today. **assay scales** — A bench of grade scales and a core sampler. A lot's grade is read
here and written on the manifest; the price is per tonne of what the scales say, not what the seller
says. **barred window** — The bonded hall: racked pallets under yellow lamps, tracked dollies, a manifest
clerk on a tether, and a freight door at the far end with a berth number beside it. No public way in.
**hatch** — An alloy hatch with a card reader and no handle on this side. YARD PERSONNEL ONLY, and under
it, smaller, THAT MEANS YOU.

*Exits.* `Exchange Stair <ES>;es;exchange;up;u` → P08 (stair, `portal.at` + dz +7 m); `Maintenance <MS>;
ms;maintenance;hatch` → P15 (hatch; `lock: staff`; visibility hidden; return `Receiving Hatch <RH>;rh;
receiving;out`).

*Items.* intake board (LookRendered: Kasei's bids, remaining intake, funded balance); assay scales
(detail only).

#### P08 Materials Exchange — Public Deck

**As built (IC8):** P08's board carries Kasei's alloy and propellant asks and the Yard's crated-fittings ask
(the fittings ask renders on both P08 and P11; IC8 (4)), each line naming its signatory. The `price board` detail
itself still names `+market/list` (Stickney v3; IC12 struck verb meta-text from the Ceres rooms the story
test walks only — an unowned residual).

*Purpose.* Kasei's asks (structural alloy stock, drive propellant) for the return run and the bids
board; `+market/list` defaults to this venue. The public rung stair down to the receiving counter is the
one cross-deck public link (`mode: stair`, `portal.at` segment + `dz_m` −7). The berth-locked freight
stair to P06 is pruned with P06 (returns in P-F).

*Short.* A trading floor the size of a cargo bay, asks on one wall and bids on the other.

*Desc.* Kasei's prices fill one wall: structural alloy stock by the tonne, drive propellant by the
tonne, each line with a quantity beside it that goes down and never, in anyone's memory here, up. The
opposite wall is bids, mostly Kasei's own for metal concentrate and water ice, a few private lines from
yard contractors in smaller type. Between the walls the floor is bare alloy scored by pallet feet, and a
rung stair in a railed well goes down to the receiving counter. The room is cold — warehouse air leaks
up the well — and smells of machine oil. The concourse is out; receiving is down the stair.

*Details.* **price board** — ALLOY STOCK, STRUCTURAL — per t — remaining. PROPELLANT, DRIVE — per t —
remaining. The figures are Kasei's and change when Kasei says so; `+market/list` has them to the credit.
Below Kasei's lines, in the Yard's own stencil: YARD FITTINGS, CRATED — PER TONNE, THE YARD'S FIGURE —
STICKNEY YARDS — SIGNED AT THE SERVICE COUNTER, with a chalked note beside it, CRATED: 2.3 CUBES TO THE
TONNE. A Dray captain who reads both boards sees the return run's arithmetic: alloy stock fills the
rating, fittings fill the bay, and the hull that leaves with both full pays best. **bids board** —
KASEI METALS BUYS: concentrate and water ice, the bid per tonne beside each line with the intake left
this cycle; the figures are the board's own, never the paint's. Under it, contractors' chits: a tonne
of alloy offcuts wanted, a Hod seat to Isidis offered. **stair well** — A
rung stair in a railed well, seven metres down to the Freight Deck, with a handline beside the rungs for
those who prefer to float it. RECEIVING COUNTER — PUBLIC; the arrow is green. **floor** — Pallet scoring
in the alloy, in parallel lines from the well to the door. Freight used to come up this way. Now it does
not, and the lines are a record.

*Exits.* `Concourse <C>;c;concourse;out` → P04 (door); `Receiving Stair <RS>;rs;receiving;kasei;down;d`
→ P07 (stair, `portal.at` + dz −7; return `Exchange Stair <ES>;es;exchange;up;u`).

*Items.* price board (LookRendered: Kasei's asks and the Yard's fittings ask, which draws from
`item.yard_spares` in P15 and is signed at P11); bids board (LookRendered: Kasei's bids with remaining
intake).

#### P09 Yard Commissary — Public Deck

*Purpose.* Provisions in crew-days at the Republic posting for the return run; the storekeeper NPC
(Okonkwo-Steyn) is the Warrant signatory of the commissary's ask, as every listing in the slice has a
signatory of its own house (doc 18 §4.3). The storefront locker that backs it sits in the Maintenance
Spine. Propellant is Kasei's board upstairs, not sold here.

*Short.* A stores counter under a Republic posting, shelves of ration crates behind.

*Desc.* Shelving in alloy racks runs back from a counter, crates of voyage provisions stacked by the
crew-day, and the Republic's posted price stencilled on a board beside the till — one figure, no haggling
line. A cold-locker hums at the back and drips into a tray. The counter has a cage over it that is only
half closed, and a hand-lettered sign gives the hours, which are the shift bells. The room smells of
cardboard and the sweet tang of ration bars and is the warmest on the deck. The concourse is out.

*Details.* **price posting** — VOYAGE PROVISIONS — per crew-day — posted by the Assembly's Office of
Supply. The figure has a date. There is no second figure. **crates** — Sealed crew-day crates, four
kilos a day: food, potable stores, scrubber cartridges. A crate opened on a shelf shows what a day looks
like. It does not look like much. **sign** — OPEN FIRST BELL TO LAST BELL. CLOSED FOR INVENTORY WHEN THE
DOOR IS SHUT. Both in the same hand.

*Exits.* `Concourse <C>;c;concourse;out` → P04 (door).

*Items.* price posting (LookRendered: the provisions ask); sample crate (detail only).

#### P10 The Shift Board — Crew Commons, Public Deck

*Purpose.* The haulers' and yard workers' mess and, after the bell, the bar: where strangers meet and
scenes happen. Settlement lore and guarded dock lore; the cook extra. Job notices on the wall are
flavour in the slice and the natural home of a later contract board.

*Short.* A mess hall that doubles as a bar after the shift bell.

*Desc.* Long tables bolted to the deck, benches with tether loops under them, and a serving hatch with
a chalk menu that has not changed in a year. After the shift bell the hatch turns into a bar. The walls
are the station's unofficial notice board — crew wanted, a bunk for rent, a Dray's worth of alloy
offcuts going cheap, a card for a dockworkers' meeting that nobody has taken down. A Settlement-era
photograph of the yard strike hangs over the hatch. The room is loud and warm and smells of fried
protein and coffee that has been on the heater since the last shift. The concourse is out.

*Details.* **notice wall** — CREW WANTED, DRAY, CERES RUN, SHARE BASIS. BUNK TO LET, QUIET, NO GUARD. And a
plain card, no name: DOCKWORKERS — THURSDAY — THE USUAL PLACE. **photograph** — The Phobos yard strike,
2309: a drydock floor full of people on tethers, holding a banner that reads OUR YARD. One young face at
the front is circled in pencil, and someone has written the word YARDMASTER beside it with no comment.
**chalk menu** — Protein, fried. Protein, not fried. Soup (ask). Coffee. After the bell the word BEER
appears under it in a different chalk. **hatch** — A serving hatch with a heat lamp over it and a tally
sheet beside it for crews who settle on payday.

*Exits.* `Concourse <C>;c;concourse;out` → P04 (door).

*Items.* notice wall (detail; readable cards); strike photograph (detail).

#### P11 Stickney Service Counter — Public Deck

*Purpose.* Hull information; the Yard's new-Dray price card as the buyout comparator under R1 (the
pasted 120,000cr is the balance reference; owning a hull is the two-weeks-of-good-runs aspiration); a
future refit contact with no repair promise; Tycho Mutual's Phobos agent merged here (doc 11 §4.7 places
an agent at Phobos); **the Yard's crated-fittings ask** (doc 18 §4.3, D-18-40: `industrial_parts` at
the Yard's posted ask, 400cr/t at seed, from the `YARD — SPARES` cage in P15), with the service clerk NPC as its Warrant signatory — the
Stickney Yards vendor's one line, read at this counter and on the P08 board. The port entry lists this
room as `yard`.

*Short.* A yard counter with a rack of hull cards and a window onto a drydock.

*Desc.* A counter of worn alloy under the Yard's stencil, with a card rack of every hull class Stickney
builds — Vigil, Hod, Anvil, Cairn, Dray — and a price card for a new Dray clipped to the front of it. A
window behind the counter frames a drydock cradle with a hull in it and a work crew on tethers under a
lamp. A board lists yard-time by the drydock-day, most lines marked RESERVED. Nothing here promises a
repair; the board says ENQUIRE. A second, smaller sign on the counter's end carries a Luna insurer's
mark. The room smells of solvent from the shop beyond the wall. The concourse is out.

*Details.* **Dray price card** — DRAY-CLASS BULK HAULER, NEW, STICKNEY BUILD — 120,000cr, delivery subject
to yard-time. The card is the figure a charter pilot measures a career against. Under it, in pencil:
"used ones exist, ask Tycho". **yard-time board** — Drydock-days by cradle. RESERVED — GUARD. RESERVED —
KASEI. One line reads ENQUIRE and that is the refit queue; no date is promised on it. **hull cards** — A
card per class with a profile drawing and a dry mass. The Dray's reads "slow, efficient, unarmed"; the
Vigil's reads "fast, expensive, visible from a long way off", which is the Yard quoting the Guard's own
brochure. **fittings card** — YARD FITTINGS, CRATED — PER TONNE, THE FIGURE ON THE CARD THIS CYCLE —
REMAINING — pressure fittings, ducting and tank sections, Stickney build, crated with dunnage for the
Belt run. In pen under it: "2.3 cubes to the tonne. Your problem, captain." The Yard's one line of
trade, signed by the clerk; the card is the listing (LookRendered), `+market/list` shows it and
`+market/quote` draws it into a docked hull. **window** — A cradle, a hull with its drive
section open, three workers on tethers and one supervisor who is not. The work is slow. It is always
slow, and it is always going on. **insurer's mark** — TYCHO MUTUAL ASSURANCE & SALVAGE — AGENT IN ATTENDANCE. Renewals, claims, the Re-float Program by
referral to Kirkwood. The agent shares the counter and the clerk's patience.

*Exits.* `Concourse <C>;c;concourse;out` → P04 (door).

*Items.* fittings card (LookRendered: the Stickney Yards ask with its remaining stock — the one place
the figure is written); Dray price card (the buyout comparator, a programme figure); yard-time board
(detail only); hull card rack (detail; five readable cards).

**As built (IC8, IC12):** the `fittings card` detail ships as IC8 re-wrote it (Stickney v2): "YARD FITTINGS,
CRATED — PER TONNE — REMAINING — pressure fittings, ducting and tank sections, Stickney build, crated with
dunnage for the Belt run. In pen under it: "2.3 cubes to the tonne. Your problem, captain." The Yard's one
line of trade, signed by the clerk; the figures under it are the Yard's own and change when the Yard says
so." — no `LookRendered` and no verb, the live figures on the board. Stickney v3 adds the details `yard time
board` and `insurers mark`, their texts as above. The `Dray price card`'s 120,000cr stays (a fiction figure
with no live render; IC12 O3).

#### P12 Yard Observation Gallery — Public Deck

*Purpose.* The quiet scene room: industrial scale, the drydocks, the Guard's pad with its Vigil cutters
(doc 11 §7), the refinery stacks and Mars. No counter, no verb; the one room built for a conversation.

*Short.* A dim curved gallery of windows over Stickney crater.

*Desc.* The gallery is a curve of floor-to-ceiling ports along the crater's rim. Below, the drydock
trusses stand in rows across the crater floor, cradles lit by work lamps, and beyond them the Guard's pad
with two cutters berthed nose-out and a third cradle empty. Past the pad the refinery's stacks vent in
slow white plumes that fall back as frost. Above, Mars fills a quarter of the sky, rust and white,
turning so slowly it seems not to. The gallery is kept dim so the view reads; the only sound is the air
plant and the tick of the ports cooling. It smells of nothing, which on Phobos is a luxury. Benches with
tether loops face the glass. The concourse is out.

*Details.* **drydocks** — Six cradles in two rows on the crater floor, trusses taller than the rim walk.
Three hold hulls; one holds a Dray with its hold open like a jaw. The Yard builds for the Republic and
"for a price, for anyone the Republic licenses". **Guard pad** — Two Vigil-class cutters, torch nozzles
capped, berthed nose-out toward the lane. A cutter leaves the pad maybe once a week, and the whole crater
watches it go. **Mars** — A quarter of the sky, Coprates somewhere under the dust, the terminator a soft
line. Phobos keeps one face to it, so it never sets. **refinery** — Kasei's stacks beyond the pad,
venting. Water ice goes in at one end; propellant and alloy stock come out the other, and the Freeport's
bank holds the paper on the whole of it.

*Exits.* `Concourse <C>;c;concourse;out` → P04 (door).

#### P15 Yard Maintenance Spine — Freight Deck, restricted

*Purpose.* The one restricted room: `lock: staff`, `portal.visibility: hidden`, `access: restricted`
(paired exits lock only the way in, `apply.rs:1500-1519`). Holds the storefront items that back the
counters above (Kasei's intake manifold and alloy/propellant store, the commissary's provisions locker)
so no `ItemDef` lock is needed, and the glassed yard office where Castellanos, Tanaka and Bask are
placed `enabled:false` for the faction cut to bind. The expansion anchor for P06/P13/P14 (P-F).

*Short.* A service corridor of cable trays and storefront cages, the yard office glassed off at its end.

*Desc.* A service corridor runs behind the freight deck, cable trays overhead, pump manifolds along one
wall, and three wire cages holding the manifolds and lockers that feed the counters above: Kasei's
intake and store, the commissary's provisions, the Yard's spares. At the far end a glassed office looks
down into the bonded hall — three desks, three dark terminals, nameplates face down. The air is cold
and loud with pumps and smells of hydraulic fluid. YARD PERSONNEL ONLY is stencilled every ten metres.
The receiving hatch is the way out.

*Details.* **cages** — KASEI — INTAKE & STORE. COMMISSARY — PROVISIONS. YARD — SPARES. Each cage has a
manifest clipped to it, and the manifests are the station's real stock, not the boards upstairs.
**office** — Three desks: the managing director's, the yardmaster's, and one with a Guard crest, kept for
a commodore who prefers the pad. The terminals are dark. The nameplates are face down on purpose.
**manifolds** — Propellant from the refinery, water from the bunker, air from the plant, each valve
tagged with a date and an initial.

*Exits.* `Receiving Hatch <RH>;rh;receiving;out` → P07 (door; unlocked this way).

*Items.* `item.kasei_manifold` (the storefront container backing Kasei's vendor vault: the warehouse pools
for the concentrate and water-ice bids and the alloy and propellant stock); `item.commissary_locker`
(backing the Yard Commissary's vault); `item.yard_spares` (the storefront container backing the Stickney
Yards vendor's vault: the crated-fittings stock, 5,400 t at seed, drawn by the ask signed at P11; the
P-F expansion anchor moves to a fourth, empty cage).

### 2.4 The cast

**Petty Officer Teodora Ilunga-Brandt** — `npc.ilunga_brandt`, P03 Republic Customs Hall, minor
(service), interfaces `atlas`, `travel`. Orbital Guard customs detail: the Republic customs officer.
*Persona:* a petty officer on the customs rotation, which the Guard treats as a punishment and she
treats as a posting. Formal, correct, faintly proud of the forms; believes the corridor is the Republic's
to keep and that a manifest tells you more than a scan does. Local, Settlement-born, has never been to
Coprates. *Speech:* clipped, procedural, every sentence a line from a form; says "declare", "attest",
"in order"; never raises her voice, lowers it instead. *Knows:* the posted charges and what each form is
for (none collected in the slice; she says so if asked, as "waived under the current notice"); the
station's layout, the locks, the concourse, which desk does what; that the Guard pickets Kirkwood from
2347 and calls it customs assertion; how to reach the harbourmaster and file a sailing plan. *Does not
know:* anything about a specific hull's cargo she has not seen on a manifest; the Guard's standing orders
at Kirkwood; Kasei's prices or funds. *Fallback:* "Declare, attest, proceed. The concourse is through the
far doors; +atlas names the rest." / "Berthing is the harbourmaster's; +ship tells you where your hull
stands. The Guard reads manifests, not minds."

**Jonas Achterberg-Nkemelu** — `npc.achterberg`, P07 Kasei Receiving Office, minor (service),
interface `market`. Kasei Metals receiving clerk, the Warrant signatory of Kasei's bids. *Persona:*
terse, assay-minded, more interested in a lot's grade than in the person selling it. Has worked the
receiving counter for eleven years and has signed for every tonne that came through it, which under the
Warrant means his name is on it. Honest about the intake left and the money left, because both are on
the board behind him and he dislikes being asked twice. *Speech:* short declaratives; numbers before
nouns; "Grade first. Then price."; looks at the scales when he talks, not at the seller. *Knows:*
Kasei's bids (concentrate and water ice per tonne as the board posts them this cycle, the intake
remaining and the funded balance; that the refinery is water-starved, which is why the ice bid stands and the ore
bid came down); that `+market/list` shows the bid and `+market/sell <listing>=<qty>` sells into it
from a docked hull's hold; that Kasei sells alloy stock and propellant upstairs at the exchange and the
Yard sells crated fittings from the service counter, "which cube out before they weigh out"; that the
refinery turns water ice into propellant and concentrate into alloy stock. *Does not know:* Kasei's debt
terms with Ceres Clearing (he knows there is a debt; everyone does); who signed a lot at Ceres beyond the
name on its manifest; anything about the Guard's pickets. *Fallback:* "We buy concentrate and ice.
+market/list, then +market/sell. The grade is what the scales say." / "Intake left and funds left are on
the board. When either reads zero, so do I." / "Ice we need. Ore we can wait for. The board says which."

**Wren Castillo-Haugen** — `npc.castillo_haugen`, P11 Stickney Service Counter, minor (service),
interfaces `help`, `market`. Stickney Yards service clerk and the Warrant signatory of the Yard's one
listing, the crated-fittings ask (doc 18 §4.3; signs only Stickney Yards' offer, never Tycho's business,
as the Warrant requires); also sits the Tycho Mutual agent's chair at the same counter. *Persona:* yard-weary, dry, competent; has heard every version of "can you just look at it" and
answers all of them with the board. Knows every hull class the Yard builds by its dry mass and its
faults. Holds the Tycho agent's referral book as a second job and resents it exactly enough to be funny
about it. *Speech:* flat, patient, with a long pause before "no"; quotes the price card as if it were
scripture and the refit queue as if it were weather. *Knows:* the five classes the Yard builds and their
profiles; that the Dray is "slow, efficient, unarmed" and cannot outrun a Vigil; the new-Dray price
(120,000cr) and that it is the figure a charter pilot measures a buyout against; that yard-time is a
reservation, not a cargo, and the refit queue has no promised date; the fittings ask (the card's figure
per tonne, crated, "two point three cubes to the tonne, so it's your problem, captain"; that the Co-op at Ceres
wants them for the fleet and the tank farm; that `+market/quote` draws them into a docked hull);
Tycho's part: renewals and claims here, the Re-float Program by referral to Assessor Adeyemi at
Kirkwood. *Does not know:* whether any particular hull can be repaired, or when; the Guard's cradle
reservations beyond the word RESERVED; the Yard's books, or the golden share's terms; what the Co-op
pays. *Fallback:* "Hull information, yard-time, fittings by the crate, Tycho renewals. Repairs: the
board says ENQUIRE, and it means it. +ship tells you what you fly." / "New Dray, a hundred and twenty
thousand. Used ones exist. Ask Tycho, who is also me, on Tuesdays." / "Fittings, the figure on the card,
by the tonne. Crated, so they cube out; that's your problem, captain. +market/list."

**Pavla Adeyinka-Rusk** — `npc.adeyinka_rusk`, P05 Harbourmaster's Office, extra. Harbourmaster's clerk:
berth status, sailing plans, the captain's minutiae. *Persona:* the harbourmaster's clerk, which is to
say the harbourmaster on every day the harbourmaster is at the pad. Brisk, exact about hours, kind to
anyone who files early and curt to anyone who files late. Keeps two clocks and resents the question
about them. *Speech:* quick, administrative, fond of "filed" and "cleared"; gives the hour in Freeport
time, then in Coprates standard, then sighs. *Knows:* which cradles are occupied, cleared, overdue or
idle; that a sailing plan is filed and amended with `+ship/order` until the burn, that the office runs a
filed plan's lines under the captain's signature, and that a docked hull with no plan is IDLE; that
`+ship/board` works from the berth gallery and `+ship/leave` from the hull's own airlock, and neither
anywhere else; the clocks, Freeport time against Coprates standard, and the offset. *Does not know:*
what any hull carries; why the Guard holds a cradle on the pad empty; Kasei's board. *Fallback:* "File it early, amend it late, sail on the hour. +ship/order.
The board does not lie; it just does not forgive." / "Your hull is on the board if it is in a cradle.
+ship says the same with fewer stamps."

**Dmitri "Mitya" Abubakar-Lange** — `npc.abubakar_lange`, P10 The Shift Board, extra. Cook at the Shift
Board; the mess, the bar, Settlement lore, guarded dock lore. *Persona:* the cook, and after the bell the
barman, and at all hours the memory of the yard. Was a strike kid in 2309, a dockhand after, a cook when
his back went. Loud, generous, sentimental about the strike and cynical about everyone who profited from
it. Knows who is in the room because he fed them. *Speech:* big, warm, interrupting himself; calls
everyone "crew"; tells the same three stories and makes them land each time. *Knows:* the strike, as he
tells it: who stood on the drydock floor and who is on the shift boards now; that the Co-op's Drays run
the Ceres lane on a share basis and the notice wall says who is hiring; which of the cards on the wall
are real and which are a year old; what the Guard pays for a cutter crew and why the yard hates it.
*Does not know:* anything he has been asked to keep, which he says is a great deal; the Yard's books;
what the Thursday card on the wall is for, out loud. *Fallback:* "Sit, crew. Protein's fried, coffee's
old, the wall's got work on it if you can read. Everything else is +market and +credit and none of my
business." / "That photo? That's the yard before it had a yardmaster. Some of us are still in it."

**Hanne Okonkwo-Steyn** — `npc.okonkwo`, P09 Yard Commissary, extra. Storekeeper of the Yard
Commissary under the Assembly's Office of Supply; the officer of record for the commissary's
provisions ask and its Warrant signatory. *Persona:* a Settlement civil servant who runs a stores
counter the way the Republic runs a customs hall: one posted figure, a dated stamp, a ledger that
balances. Counts crew-days the way Akintola does at Ceres, with less patience and more initialling.
Will sell a captain exactly what the plan says and not a crate more without a signature. *Speech:*
clipped, numbered, Republic typeface spoken aloud; "posted", "initialled", "per crew-day"; the shift
bells are her clock. *Knows:* the posted price per crew-day and its date; the crates remaining, which
she signs for every shift; that `+market/list` shows the posting and `+market/quote`/`+market/accept`
fill a docked hull's locker; that two aboard need twice the days and the plan's `fill` buys the
passage's days; that the price is the Assembly's, not hers. *Does not know:* Kasei's board; the Guard's
business; anything the Shift Board's wall says. *Fallback:* "One figure, posted, dated. +market/list,
quote it, accept it; I sign for the crates and you sign for the days." / "Closed for inventory when
the door is shut. It is not shut. Crew-days, how many?"

**The principals**, P15, major, `enabled:false`, no interfaces, not voiced in the slice: **Managing
Director Priya Castellanos**, Kasei Metals ("serves two masters and is precise about which one she is
serving on any given day", 11 §4.3); **Yardmaster Ruaridh Tanaka**, Stickney Yards ("a strike leader in
2309, an oligarch in 2350; wants the Yard busy and the Guard buying", §4.2); **Commodore Teodor Bask**,
Orbital Guard, Phobos ("the hawk; believes one seized hauler ends the argument", §3.2).

### 2.5 Lore pool (tags `corridor_trade`, `port_stickney`, `tycho`)

*Public.* **Stickney Yards** — Phobos's drydocks, made "a real yard" by the Republic in the 2340s (doc 11
§2). The Yard builds Vigil cutters, Hod dropships, Anvil mechs and Cairn GCVs for the Republic "and, for
a price, for anyone the Republic licenses", and sells yard time, "a reservation, not a cargo" (§4.2). It
is state-adjacent: the Republic holds a golden share. **The Dray** — every Dray in the corridor was
built here: "Slow, efficient, unarmed. Cannot outrun a Vigil; can choose a burn that makes the intercept
cost more propellant than the Guard wants to spend" (§7). The Co-op, Lantern and Tycho's loaner pool fly
them. A new one is a hundred and twenty thousand credits on the service counter's card. **Kasei Metals**
— the corridor's one refinery, at Phobos: "metals → structural alloys, volatiles → propellant" (§4.3).
Kasei buys Ceres concentrate and water ice at the receiving counter and sells alloy stock and propellant
at the exchange. Its intake and its funds are finite and posted; when either reads zero the clerk stops
signing. The ice it buys is Phobos's water — the refinery's process water, the Yard's and the
habitat's, and the Republic's reserve — which is why the bid stands above anything the stacks alone
would pay; its reaction mass sells at a Republic-posted price, a strategic asset the Republic keeps
fuelled for the Guard and the Yard, dearer than Occator's crude and unrelated to the ice bid (doc 18
§4.3). **The Orbital Guard** — the Republic's small space force: "torch-drive Vigil-class cutters based
at Phobos; the corridor's interceptors" (§3.2). The cutters are "fast, expensive, visible from a long way
off" (§7). Two sit on the pad below the observation gallery; the Guard's customs detail staffs the hall.
**Customs assertion** — from 2347 the Republic "stations customs pickets at Kirkwood" (§2). A Dray on
the Ceres lane passes a Republic picket's transponder asking for its manifest at the Kirkwood leg. The
posted charges in the customs hall are the policy's paperwork; in the slice nothing is collected. **The
Settlement** — "A decade of tax refusal and the Phobos yard strike; two years of intermittent shooting in
orbit" ended with the Republic of Mars recognised in 2312, inheriting Concord-era treaties and debt
(§2). The strike photograph over the Shift Board's hatch is the yard's founding document; the Yardmaster
is in it. **The Exchange compound** — the Malapert Exchange runs the Phobos relay half from its own
compound under "the 2312 Settlement's extraterritoriality clause, inherited by the Republic" (doc 15
§4). Every message to Ceres goes through Luna's queue. Standard traffic waits by the congestion board;
the board overstates the truth and "the gap is the upsell" (doc 15 §5.2).

*Guarded.* **Who holds the refinery** — Kasei was "built with Ceres Clearing money; the debt gives the
Freeport's bank economic control of a Republic strategic asset, and the Republic knows it" (§4.3).
Phobos's economic control is shared, Republic and Ceres Clearing (§5). The Republic wants "its Phobos
refinery out from under Belt debt" (§3.2); the receiving clerk's funds are, in the end, Freeport money.
**The Republic's wants** — "The Concession ended and Isidis administered from Coprates; customs authority
over the Kirkwood approach; its Phobos refinery out from under Belt debt; a fleet; a fab of its own,
which the Compact forbids. It has the army and the cutters, and lacks the water, the money, and the legal
case" (§3.2). The Concession Review clock stands at 2 of 6 (§6). **The mark** — "The Republic has floated
pricing Phobos yard-time in a Martian mark — a political act, not a monetary one" (§1). The yard-time
board still reads in credits. Ask the service clerk which column comes first on the next printing and
get the long pause.

*Secret* (stored as a Secret, never in a prompt). **The hawk** — Commodore Bask "believes one seized
hauler ends the argument" (§3.2). The empty third cradle on the Guard's pad is kept empty for a reason,
and the Customs Assertion clock advances on "each successful interdiction at Kirkwood" (§6). A
Freeport-flagged Dray is the hauler he means. **The tap** — the Office of Territorial Security "wants a
tap" on the Phobos relay half; "whether it has one is a seed-pack secret" (doc 15 §4). OTS is also
"trying to buy" the Tether (§4.6). The Thursday card on the Shift Board's wall is watched by someone who
is not a dockworker.

### 2.6 Ambience

The shift bell sounds twice, and the handline traffic on the concourse reverses direction. · A pressure
door cycles somewhere toward the berths; the counter above it clicks over. · Pump vibration from the
refinery rises through the deck, holds, and settles back into the boot-cleats. · A work lamp on the
crater floor goes out, and a moment later another comes on two cradles over. · The approach-control
channel mutters a cutter's callsign and a bearing, then returns to static. · Frost from the refinery
stacks falls back across the pad in a slow white curtain. · Someone pins a new card to the Shift Board
wall and takes an older one down without reading it. · The commissary's cold-locker drips into its
tray, once, and the hum changes pitch. · A dolly alarm chirps in the bonded hall and stops. · Mars has
turned by a degree that nobody noticed, and the terminator has moved onto Coprates. · A Guard recruiting
bill loses a corner to the ventilation and curls a little further. · The berth board updates: one cradle
goes from IDLE to CLEARED, and the room's attention goes with it.

---

## 3. The Dray's interior

Seven rooms ride every hull (doc 18 §4.2, D-18-22, D-18-29): six walkable members of one interior
space whose object is the Airlock, and one unwalkable store that backs the hold vault. The template
lives on the unit definition (`tacsim.unit.dray` → `interior`, doc 18 §4.2's JSON) and the tacsim
`PortApplier` instantiates it per hull: seven Rooms, five exit pairs, `sim.interior` on the airlock and
the stowage, `Transition::Space` + `Transition::Members`, the `berths` compartment (max 2). Rooms keep
plain names; the hull is named by the `look` status line that tacsim adds on the atlas `look_rendered`
precedent (`travel.rs:2673-2695`): "Aboard the *Salt Moth* (FPC-D-0401), docked at Ceres Freeport" or
"under way, leg 2 of 2, Kirkwood astern, ETA 15:06 UTC" (the status line and the `+ship` renders are
the OOC-facing surfaces that speak real time; room text never does). **Orientation and authority:**
the airlock is **forward** of the Central Passage and the Crew Compartment **aft**; doc 18 §4.2's
`interior.exits` JSON is the authority and §3.1 below matches it alias for alias. Each room's gauges read the same pools
`+ship/hold` reads (the cargo load cells in the hold, in tonnes and cubic metres against both limits;
the tank gauges in engineering; the locker tally in the crew compartment; and all three on the Central
Passage's manifest slate and the Bridge's displays), so the ship tells the pilot where they are and how
they stand without a verb. **Every such readout is a `look_rendered` line from `interior.gauges`**
(doc 18 §4.2's JSON, a pool → rooms map), rendered from the pools when the room is looked at; no
`RoomDef.details` text aboard carries a load, a tank level or a tally, because a detail is static pack
text and would read the same on every hull for ever (doc 18 D-18-39 forbids a stored 20/80 anywhere).
Walking between rooms is ordinary exit traversal under the boundary rule; the only ways on and off are
`+ship/board` in a berth gallery and `+ship/leave` in the Airlock, docked. Under way nothing crosses.
Theme: the Freeport's base and vocabulary, plus the ship's own: lock, hatch, ladder well, couch, ROM,
manifest slate, load cells, tankage, reaction mass, bunk, still, crew-day, tally, packed volume, cube,
rated.

**As built (IC4, IC6, IC8, IC9):** the template ships in `tacsim.pack.dray` **v2** (IC8): every room's prose
is **figure-free** — no load, no limit figure, no engine word (the `load cells` say "a LIMIT line", the `tank
gauges` "against the tank's rating", the `displays` "the ETA"; IC8 (6), stricter than "strip the
meta-text") — and every figure is a `look_rendered` gauge line from `logistics` under the room's
instrument (`Load cells`, `Manifest slate`, `Displays`, `Locker tally`, `Tank gauges`;
`tacsim/src/port/gauges.rs:67`). The Airlock's alias **`lock` is dropped** (it shadowed the cockpit verb
LOCK in the Central Passage; exits match before commands), so the passage's way forward is `Airlock
<A>;airlock;forward`. The template's **`interior.exits` are append-only by position**
(`tacsim/src/port/hull.rs:22`): an exit's position is its identity across a re-dress. A chartered hull
Thing with no description takes `HULL_DESC_TEMPLATE` ("A Dray-class bulk hauler, {mark}, cradled with its
umbilicals slack: blunt, grey and webbed with reaction-mass hoses.", `tacsim/src/port/coop.rs:102`; IC6).
The status line carries the registry mark in every hull room (IC6 L18; §3.2 as built).

### 3.1 The rooms

| id | Room | Kind | Exits | What the pilot sees of the compartments |
|---|---|---|---|---|
| `airlock` | Airlock | walkable; the space object | inner hatch → passage | the outer door's seal light reads the berth's state |
| `passage` | Central Passage | walkable | airlock, up → bridge, aft → crew, down → hold | the manifest slate: cargo by good, in t and m³ |
| `bridge` | Bridge | walkable | down → passage | course, tank, locker and hold readouts; the status line |
| `crew` | Crew Compartment | walkable | out → passage | the stores locker's tally (crew-days) |
| `hold` | Cargo Hold | walkable | up → passage, down → engineering | the load cells: tonnes and cubic metres by good against 100 t and 120 m³ |
| `engineering` | Engineering Access | walkable | up → hold | the tank gauges: reaction mass against 80 t, the reserve after the planned burn |
| `stowage` | Hold Stowage | store; no exits; the vault backing | — | not a place anyone walks |

#### Airlock

*Purpose.* The space object (it carries `sim.interior`); the only boarding point. `+ship/board` from a
berth gallery places the pilot here; `+ship/leave` from here returns them to the gallery, docked. The
outer door is a detail, never an exit: it reads sealed under way and "open to the gallery" when docked.

*Short.* A two-person lock, scuffed white, a suit locker and a pressure gauge.

*Desc.* A two-person lock, scuffed white, with a suit locker and a pressure gauge whose needle never
quite settles. The outer door carries a Stickney Yards builder's plate and a Co-op registry mark
stencilled over an older one; its seal light reads the state of the berth beyond. Grab rails are worn
to bare metal at hand height. The air is the station's or the ship's depending on which door cycled
last, and smells faintly of both. The inner hatch stands open on the passage.

*Details.* **outer door** — The seal light: green when a berth gallery is on the other side and the
clamps are on, red and steady under way. Beyond it, under way, there is nothing but the dark and the
ship's own running lights. **builder's plate** — STICKNEY YARDS, PHOBOS, DRAY-CLASS BULK HAULER, a hull
number and a year. Under it a second stencil, newer, the Co-op's linked hands and a registry mark.
**suit locker** — Two suits, inspected and tagged, and the space for a third. **pressure gauge** —
Station-normal, ship-normal, and a hand-painted line between them that somebody labelled "fine".

*Exits.* `Inner Hatch <I>;inner;in;passage` → Central Passage (return `Airlock <A>;airlock;lock;forward`).

**As built (IC8):** the return is `Airlock <A>;airlock;forward` (`lock` dropped); the detail `builder's plate`
ships as `builders plate`.

#### Central Passage

*Purpose.* The spine; ladders up to the Bridge and down to the Hold, a hatch aft to the Crew
Compartment; the manifest slate (cargo by good from the hold pools, in tonnes and cubic metres).

*Short.* A low spine of grab-rails and cable trays, hatches opening off it.

*Desc.* The Dray's spine: a narrow pressurised run of grab-rails and cable trays, with a ladder well up
to the bridge and a deck hatch down to the hold, the crew compartment's hatch aft and the lock forward.
Cargo tie-points stud the deck along the whole length. The hum of the drive is felt through the
handrails more than heard, and the air moves with a steady draught from the scrubbers. A manifest
slate is clipped by the bridge ladder, its figures the hold's own.

*Details.* **manifest slate** — The hold's contents by good, in tonnes and in cubic metres, two columns
with the hold's two limits at their feet, and the tank and the locker under a line; the slate reads what
`+ship/hold` reads. **tie-points** — Recessed rings every metre, for
the day the passage is cargo too. **cable trays** — Power, data, air, in colour-banded runs; a tag on
each says which deck plate to lift.

**As built (IC8):** `tie-points` ships as `tie points`; the slate's figures are the `Manifest slate` gauge
lines, not detail text.

*Exits.* `Airlock <A>;airlock;lock;forward` → Airlock; `Bridge <U>;bridge;up;u` → Bridge (ladder well);
`Crew Compartment <C>;crew;galley;bunks;aft` → Crew Compartment; `Cargo Hold <H>;hold;down;d` → Cargo
Hold (deck hatch).

#### Bridge

*Purpose.* The departure, plan and status surface: `+ship/plan`, `+ship/undock` and the queue are usable
from anywhere aboard, but the Bridge carries the readouts, the drive ROM's pane and the `look` status
line; the natural place to file the day's orders before logging off.

*Short.* Two couches before a curve of displays, the drive ROM's amber pane among them.

*Desc.* Two acceleration couches face a curve of flat displays: course and ETA, tankage, the stores
locker, the hold manifest, the berth link. The drive ROM's amber pane sits under a sealed plate and
reads CLASS 0 — FLIES THE PLAN, DECIDES NOTHING. A clipboard holds a paper copy of the sailing plan,
because the dockmaster likes a signature; a Co-op checklist is clipped beside the drive panel. The air
is cooler here and smells of warm electronics; the only sound is the fans and, docked, the tick of the
berth clamps settling.

*Details.* **displays** — COURSE: the route, the leg, the ETA in IC and real time, docked or under way.
TANKAGE: reaction mass aboard and the burn the plan needs. STORES: crew-days against the passage.
HOLD: tonnes and cubic metres by good against both limits, FULL lit under whichever reads it, or both.
BERTH: the gallery's name, or SEALED. **ROM pane** — CLASS 0. It flies a filed
sailing plan to the second and contracts for nothing; the signature on the plan is the captain's, and
under the Warrant so is the liability. **sailing plan** — A paper copy of `+ship/orders`, the orders
numbered, the departure hour circled. **checklist** — BEFORE CLEARANCE: tank ≥ burn. Locker ≥ days.
Payload under the rail. Crew of record aboard. Plan filed. Someone has ticked every box in pen and then
added "sleep".

**As built (IC8):** the `displays` detail reads "the ETA" (no time base named in room text), and the
`sailing plan` detail reads "A paper copy of the sailing plan, the orders numbered, the departure hour
circled." — the brief's "paper copy of `+ship/orders`" (a verb as meta-text) is not shipped. The Bridge's
burn-and-reserve gauge lines are not built (IC11 L21; unowned).

*Exits.* `Passage <D>;passage;down;d` → Central Passage.

#### Crew Compartment

*Purpose.* Bunks, galley and the stores locker; the natural place to log off and hold scenes aboard.
The locker's tally shows the provisions pool. The `berths` compartment is two: the pilot and one
passenger or relief (reserved in the slice).

*Short.* Two curtained bunks, a fold-down galley, a water still that ticks as it cools.

*Desc.* Two bunks behind privacy curtains, a fold-down galley table with a two-ring heater, and a water
still that ticks as it cools. The stores locker is set into the bulkhead, its tally card in a plastic
sleeve beside the hatch. Someone has taped a Kirkwood Depot shift roster above the left bunk and never
taken it down. The air is warmer and softer than the passage and smells of coffee, sealant and the
vanilla note of ration bars; the only place aboard that smells of anything but metal. The passage is
forward.

*Details.* **locker tally** — Crew-days aboard, the passage's need beside it, and a line in the chandler's
hand: TWO ABOARD, TWICE THE DAYS. **bunks** — The left one has a bag on it and a reading lamp; the
right one is made with Co-op corners and has never been slept in. **still** — Station water in,
ship water out, a tick every few seconds as the coil cools. **shift roster** — Kirkwood Depot, a month
old, with one name circled and no reason given.

*Exits.* `Passage <P>;passage;out;forward` → Central Passage.

#### Cargo Hold

*Purpose.* A walkable catwalk among the bulk bins; the load-cell panel shows the cargo pool's tonnes and
cubic metres by good against the hold's 100 t and 120 m³ (doc 18 §4.3, D-18-28 as replaced by R7).
Never a vault backing (the stowage behind the frames is).

*Short.* A tall, cold bay of bins and lashing rails, lit blue, the load cells glowing by the hatch.

*Desc.* A tall, cold bay of bulk bins, canister racks and lashing rails under blue work light, crossed by
a catwalk from the deck hatch to the engineering ladder. The load cells' readout glows on the forward
bulkhead: tonnes and cubic metres by good, and the hold's two limits under them. Frost forms on the ice
canisters and nowhere else; the concentrate bins are dusted grey; yard crates, when there are any, take
the bay the bins leave. The air is dry and tastes of metal, and every footfall on the catwalk rings
down into the stowage frames below. The passage is up; engineering is down.

*Details.* **load cells** — One line per good the hold accepts, tonnes and cubic metres side by side,
a TOTAL line under them and LIMIT 100 t / 120 m³ under that, with FULL lit under whichever limit reads
it, or both; cubic metres to a tenth, truncated, never rounded up. The figures are the cargo pool's
own, rendered when the room is looked at (the `cargo` gauge of `interior.gauges`, a `look_rendered`
line, not this text), so no two hulls and no two days read alike and nothing here carries a load.
**bins** — Strapped bulk containers in stowage frames, numbered; behind them the racks
nobody walks, where the cargo physically is. **racks** — The canister racks fill before the bins weigh
out: ice runs out of room, concentrate runs out of rating. **crates** — Yard crates, mostly air and
dunnage; four of them fill a bay a single bin of concentrate outweighs. **lashing rails** — Rated and
stencilled; a captain who overloads the hold will not get past the rail, and the cells say so first.
**frost** — On the water-ice canisters only. It is how a crew tells ice from concentrate in the dark.

*Exits.* `Passage <U>;passage;up;u` → Central Passage; `Engineering <E>;engineering;eng;down;d` →
Engineering Access.

#### Engineering Access

*Purpose.* The crawlway under the hold to the drive's service panels and the propellant manifold; the
tank gauges mirror the propellant pool and show the reserve after the planned burn.

*Short.* A cramped, warm, loud crawlway of pump housings and tank gauges.

*Desc.* A crawlway under the hold: pump housings, the reaction-mass feed manifold and its isolation
valves, the drive's service panels behind warning stencils, and a row of tank gauges along the aft
bulkhead. It is warm and loud, the drive's hum a pressure in the ears, and the air smells of lubricant
and hot insulation. A stencil on the reactor shield's bulkhead says not to lean on it. The hold is up the
ladder; there is no other way out.

*Details.* **tank gauges** — Reaction mass aboard against 80 t, and under it BURN FOR THE FILED PLAN and
RESERVE AFTER. Carrying reserve costs reserve; the gauge does not say so, but the plan does. **manifold**
— The feed from the tankage to the drive, with isolation valves tagged by date. The bunkering hoses
connect out here, at the berth, on the schedule. **service panels** — Sealed, stencilled DRIVE —
STICKNEY YARDS — NO USER-SERVICEABLE PARTS, and under it, smaller, in pen, "ha". **shield bulkhead** —
Warm to the touch. DO NOT LEAN. Somebody has.

**As built (IC8, IC11):** "against the tank's rating" in place of "against 80 t"; the burn-for-the-plan and
reserve-after figures as gauge lines are not built (IC11 L21; unowned).

*Exits.* `Hold <H>;hold;up;u` → Cargo Hold.

#### Hold Stowage (store)

*Purpose.* A seventh Room with no exits, its own interior space anchored to the hull, and the backing of
the single hold vault (`CustodyRef::SpatialInterior(stowage)`): the cargo, propellant and provisions
pools live here. Not walkable, so no person can become vault custody. Its desc exists for staff and for
`@vault/show`.

*Desc.* The bulk bins, the tankage and the stores locker behind the hold's frames and the compartment
bulkheads: racks, pipes and sealed cases, reached by panels and hoses, not by hatches. The frames are
the volume: a hundred and twenty cubic metres of rated bay behind the cells, which is the second limit
the load cells read beside the hundred tonnes. Not a place anyone walks.

### 3.2 What the pilot sees

Aboard, `look` is the core render of the member room plus the status line; speech stays per room; a
ship-wide narration (the lock sealing, Kirkwood, the arrival) loops over every member room. `+map`
refuses aboard ("You are not on a map": ship rooms have no `RoomGeo`; D-14-5's local carrier frame is a
later cut the template does not foreclose). `+ship` is the aboard status (hull, port or leg, ETA in IC
and real time, the hold), `+ship/hold` the pools, `+ship/plan` the dry run, `+ship/orders` the queue.
WHO shows room names to staff and to co-located players; with eight hulls sharing the same room names
the status line is what tells them apart. At the station the hull Thing appears in the berth gallery's
contents by projection; its occupants are invisible from outside.

**As built (IC6, IC9, IC11):** the status line shows in every hull room on `look`, with the registry mark,
in five forms (`tacsim/src/port/ship.rs:85-100`): "Aboard the {name} ({mark}), docked at {port}." · "…,
under way, leg {n} of {m}{astern}, ETA {hhmm} UTC." · "…, cleared for {port}." · "…, docked at {port};
departs {hhmm} UTC (in {span})." · "Aboard the {name} ({mark})." Then the room's gauge lines under its
instrument word, or "{Instrument}: dark." when a pool cannot be read. `+ship`'s first form is three lines:
"{name} ({class}): docked {port}, {gallery}." with the gallery's full name, the hold, and "Programme: {n}
sales; buyout {buyout}." (`ship.rs:104,509`; IC6 L19); under way it names the leg and the arrival in IC
days with the real UTC hour. The sailing-plan forms (C14) and pad (P05) ship as details (above).

---

## 4. Names (Q-18-12, accepted 2026-10-15)

Accepted "names ok" under R8 (doc 18 §8); binding.

**As built (IC1–IC12):** every name below shipped as written. Seven detail keys shipped respelled, five in
the stations and two aboard (§0.2 as built); no person, org or place name changed.

- **Hulls** (the Trust's pool of eight, registry marks FPC-D-0401 to FPC-D-0408; the Freeport Registry's
  Belt mirror), by their **laid-up** names: *Slack Water* (0401, the hull the first pilot on a fresh
  world is chartered: `next_loaner` takes the lowest free id), *Cold Comfort*, *Ninth Ledger*, *Patience
  Wen*, *Kirkwood Lamp*, *Long Division*, *Dry Season*, *Tally Ho*. A pilot names their hull at
  `+coop/enrol` (the transcripts' pilot chooses *Salt Moth*); the registry mark stays.
- **Ceres cast** (seven): Ilse Oduya-Brandt (Co-op desk officer and Registry duty clerk, C05); Nkem
  Halvorsen (Ceres Clearing teller, C06); Rosalind Qureshi-Mbeki (Occator factor and Warrant signatory
  of Occator's offers, C07); Margit Søndergaard-Achebe (the Freeport Exchange's factor and signatory of
  the Exchange's ask and Freeport Works' bid, C07); Dov Akintola (Co-op chandler and signatory of the
  provisions ask and the Co-op's yard-fittings bid, C08); Corin Mbatha (Tycho Mutual agent, C09, Draft 0.1's name); Yusra "Ma" Petrossian
  (the Slack Line's barkeep, C10). Principals from doc 11: Tomasz Wen, Eskil Varga, Halvard Ndiaye,
  Ludmila Osei-Ferrante, Dagny Holm (C16), Matthias Kalu (C05).
- **Stickney cast** (six): Petty Officer Teodora Ilunga-Brandt (Orbital Guard customs, P03); Jonas
  Achterberg-Nkemelu (Kasei receiving clerk and signatory, P07); Wren Castillo-Haugen (Stickney service
  clerk, signatory of the Yard's crated-fittings ask, and Tycho's chair, P11); Pavla Adeyinka-Rusk (harbourmaster's clerk, P05); Hanne Okonkwo-Steyn
  (Yard Commissary storekeeper and signatory, P09); Dmitri "Mitya" Abubakar-Lange (the Shift Board's
  cook, P10). Principals from doc 11: Priya Castellanos, Ruaridh Tanaka, Teodor Bask (P15). Thirteen
  cast NPCs at the two corridor stations, six of them signatories; with the moon stations below,
  thirty-four cast NPCs across the five, fifteen of them signatories, and eighteen principals — fifty-two
  NPCs in all. Every listing in the lane has a signatory of its own house.
- **Places**: the Freeport Hub (Ceres' structure), the Yard Rim (Stickney's), the Slack Line, the Shift
  Board, the Port Authority Gallery, the Yard Maintenance Spine.
- **Institutions and ids**: the Co-op First Captain Trust (`coop_trust`); the First Captain programme
  (`coop.first_captain`); the Freeport Exchange / Freeport Works (`freeport_exchange`; new to doc 11
  §4, the Port Council's commodity floor, doc 18 §10); Yard Commissary
  (`yard_commissary`); Stickney Yards as a vendor (`stickney_yards`, the crated-fittings ask from
  `item.yard_spares`); the good `industrial_parts` "yard fittings, crated"; `system:tacsim.treasury`;
  `tacsim.registry`; the lane pack `tacsim.lane.ceres_phobos`; the drive class `coast_hauler`.
- **Verbs**: `+coop`, `+coop/enrol`, `+coop/statement`, `+coop/buyout`, `+coop/release`; `+ship`,
  `+ship/board`, `+ship/leave`, `+ship/hold`, `+ship/plan`, `+ship/undock`, `+ship/orders`,
  `+ship/order`; `+credit`; `+ic`; staff `@coop/revoke`, `@tacsim/port status`.
- **Ship rooms**: Airlock, Central Passage, Bridge, Crew Compartment, Cargo Hold, Engineering Access,
  Hold Stowage.

**Appended with cut A.8 (2026-10-19).** The names below are the A.8 design's, authored in the moon packs
and lane v8 and checked against doc 11's principals and the shipped cast (the design's names pass, Q11).
They are **not yet accepted**: the owner's "names ok" under Q-18-12 is owed for every one of them (the
design's O11), with the history rows they carry (the ark moorings, the 2308 blockade depot, the Saturn
Charter of 2291).

- **Aitken cast** (nine): Lorcan Mwangi-Eriksen (Cabeus Works sales agent, signatory of the alloy ask and
  the concentrate bid, L07); Thandiwe Albrecht-Sato (Lunar Habitat Board receiving commissioner, signatory
  of the ice and billet bids, L08); Anneliese Okafor-Thistlewood (Crisium Instruments export factor,
  signatory of the boards ask, L07); Imogen Batista-Okeke (Highport Victualling Office purser, signatory of
  the reaction mass and provisions asks, L09); Dockmaster Aurelio Kowalczyk-Amadi (Concord Orbital Access,
  L12); Deputy Registrar Kwabena Holt-Iversen (the Registry's highport annex, L05); Mireille
  Devereux-Anand (Concord currency clearing, L06); Maren Ferro-Wainwright (the annex's export clerk, L05);
  Ngozi Rasmussen-Oto (the Cold Trap's keeper, L10). Principals: Cosima Ferreira-Lund, Odile
  Kwarteng-Fairweather, Benedikt Rourke-Haddad, Ingrid Moravec-Ansah (L15).
- **Voltaire cast** (six): Caspian Abiodun-Fleck (Reserve quartermaster, signatory of the rotation sales
  and the ice and billet bids, D07); Emeka Strand-Villanueva (Syrtis Bonded Imports buyer, signatory of
  the alloy and boards bids, D08); Femi Rautio-Grant (Republic customs inspector, D03); Aurek Szabo-Mensah
  (roadsmaster, D05); Marisol Kettunen-Ade (the Long Straw's keeper, D10); Wilhelmina Tsai-Barros (the
  Titan Concession Company's agent at the Roads, D10). Principals: Radmila Delgado-Mertens, Anders
  Bergström-Nwachukwu, Liesel Okwuosa-Brandão (D15).
- **Ligeia cast** (six): Ysolde Marangoni-Teague (Company sales clerk, signatory of the billet, ice and
  reaction mass asks, T07); Barnabas Oyelowo-Krauss (Company Store storekeeper, signatory of the
  provisions ask, T09); Saoirse Tupou-Lindahl (Selk Townships Board purchasing delegate, signatory of the
  alloy and boards bids, T08); Kenji Abara-Vos (dockmaster, T05); Noor Adichie-Brun (Lift Marshal, T03);
  Ottoline Quayle-Ibekwe (the Ninety-Four's keeper, T10). Principals: Augustin Varela-Nwosu, Hallgerd
  Asante-Pryce (T15).
- **Places**: Aitken Highport, the Aitken Drum (its site), the Hub Deck and the Drum Deck, the spoke, the
  Cold Trap, the Rim Service Spine; Voltaire Anchorage, Voltaire Roads (its site), the Roads Deck and the
  Tank Deck, the Ark Moorings Gallery, the *Long Mercy*, the Long Straw, the Reserve Spine; Ligeia
  Terminal, the Ligeia Spar (its site), the Company Deck and the Cargo Deck, the Lifter Gallery, the
  Ninety-Four, the Spar Service Spine.
- **Institutions and ids**: Concord Orbital Access (`orbital_access`), Cabeus Works (`cabeus_works`), the
  Lunar Habitat Board (`habitat_board`), Crisium Instruments (`crisium_instruments`), the Highport
  Victualling Office (`highport_victualling`); the Voltaire Bond Authority (`voltaire_bond`), the Republic
  Strategic Reserve (`strategic_reserve`), Syrtis Bonded Imports (`syrtis_bonded`); the Titan Concession
  Company (`titan_concession`), Concession Stores (`concession_stores`), the Selk Townships Board
  (`selk_townships`); the Saturn Charter of 2291; the faction words `company` and `townships`; the port
  entries `luna_port`, `deimos_port`, `titan_port` at the locations `aitken_highport`,
  `voltaire_anchorage`, `ligeia_terminal`; the goods `instrument_boards` "instrument boards, crated" and
  `carbon_feedstock` "carbon billet, pressed"; the routes `ceres_aitken`, `aitken_voltaire`,
  `aitken_ligeia`, `ligeia_aitken`, `ligeia_voltaire`, `voltaire_stickney`.

---

## 5. Aitken Highport, Luna

`region.aitken_highport` (surface region "Luna", body `luna`; locality `locality.aitken_highport`, kind
`port`) + `neighborhood.aitken_highport` v1 (requires `region.aitken_highport`@1); site `site.aitken_drum`
"Aitken Drum" (`site.transit`) and structure `structure.aitken_drum`, Hub Deck index 0 (z 0 m, 6 m) and
Drum Deck index 1 (z 6 m, 4.5 m). Twelve rooms, two decks, no Limbo door: reachable only by flight. Lane
v8's `luna_port` (location `aitken_highport`, venue `luna`) names it; every name below is the A.8 design's
and awaits the owner's "names ok" (§4).

### 5.1 Theme (`neighborhood.aitken_highport`; the 2350 base on `region.aitken_highport`)

**Name.** TacSim 2350: the Highport.

**Style guide** (the pack's, condensed). Present tense, no second person, no named NPCs in a room desc;
each room distinct in sound, light and smell before furniture. The Highport has two physics and says so
(doc 10 §3: no artificial gravity beyond thrust and rotation): the hub stands still and holds nothing
down — handlines, hook pads, tether clips — and the drum spins at the Moon's own sixth of a g — long slow
strides, slow pours, a floor that curves up ahead and behind. The Concord sounds institutional and old:
seals, brass plates, Shackleton time, the Registry's patience, habitat discipline in every poster. The
houses sound like houses: Cabeus and Crisium price lines, the Habitat Board's conservation slogans, permit
sleeves. Power shows as paperwork, as the hooded lamps in the yards' slips, and as who may open the service
hatch. Earth comes past the viewports once a turn and the basin wheels below. Desc length medium (60–120
words); details one or two dry sentences with a fact in them.

**Vocabulary.** hub, drum, spoke, spoke ladder, spin, bearing race, handline, hook pad, tether clip,
cradle, boarding collar, pressure door, sailing board, Orbital Access, the Concord, Shackleton, Malapert,
Aitken, basin, cold trap, regolith, closed loop, habitat discipline, Habitat Board, lander, freight
collar, intake, receiving, assay, Cabeus, lunar alloy, Crisium, instrument boards, crated, export permit,
Registry, annex, clearing, dry money, victualling, reaction mass, crew-day, tonne, in a good season.

**Banned.** warp, hyperspace, FTL, teleport, alien, android, gravity plating, shields, stardate, hangar
bay, you see (the region base carries the first seven and "you see", with `desc_length` medium and the
pov rules).

### 5.2 Overview

Aitken Highport is the Concord's civil anchorage in low polar orbit, named for the South Pole–Aitken
basin it crosses: Shackleton and Malapert swing under the hub ports once an orbit, and the lunar yards'
restricted slips trail it on the same truss. A Dray cannot lift from Luna's sixth of a gravity, so nothing
in this trade lands: ships berth at the drum's non-rotating hub, crews pull along the berth gallery and
climb the spoke ladder down into spin, and the drum turns at the Moon's own sixth of a g, so the Drum Deck
walks like Shackleton. The light is the polar Sun skimming the basin rim through the hub ports, and Earth
swinging past the drum's viewports every turn. It smells of habitat discipline: filtered air, hydroponic
green from the Habitat Board's racks, warm instrument dust from the clearing house, and new carpet, which
the Belt calls the smell of dry money.

The station is two decks of one structure. The Hub Deck stands still: the Visiting Berth Gallery along
the hub cradles, the Pressure Lobby at the spoke's hub end, the Dockmaster's Office with the sailing
board, and Habitat Board Receiving, where the Board's landers mate and the Belt's ice and concentrate are
sold. The Drum Deck spins: Orbital Access Arrivals at the spoke's drum end, the Drum Concourse, the
Registry Annex where export permits are stamped, the Clearing House Window, the Cabeus Exchange where
lunar alloy and crated instrument boards are sold, the Victualling Office for reaction mass and stores,
and the Cold Trap, the social room. A public stair runs a second spoke from the exchange down to
receiving. One restricted room, the Rim Service Spine, is staff-locked and hidden off receiving; it holds
the storefront items and the dark desks of the houses' principals.

Its trade is the Belt's door to the Concord. The Belt's concentrate and ice come up no further than the
Highport and go down by the Board's landers; lunar alloy and the basin fabs' boards leave by Dray for
Deimos and Titan, the only two destinations the dockmaster files (Deimos a week or two in a good season,
Titan nearer two months, one soul and throttled). Hulls arrive from Ceres and from Titan. Nothing goes
back to Ceres: nothing there bids a board, Kasei already fills the Freeport's alloy intake, and a
Ceres-bound Co-op hull calls at Kirkwood by charter. The governance is the Concord's, all four control
kinds: Concord Orbital Access is the port authority (an org with no vendor, the Ceres Clearing
precedent); the Registry's highport annex and Concord currency clearing keep desks here as rooms and desk
officers without an org of their own, on the C05 precedent; Concord Fleet's yards adjoin and are closed to
the trade; Malapert's coverage reaches the drum, and Titan is over an hour away by light, up to an hour
and a half. Every org carries the faction `concord`.

### 5.3 The rooms

| id | Room | Deck | Kind | Access | Port role | Short (the pack's `short_desc`) | Details (`look` keys) |
|---|---|---|---|---|---|---|---|
| `room.l01_berth_gallery` | L01 Visiting Berth Gallery | Hub (0) | indoor | public | gallery; `port.dock` | A long weightless gallery along the hub's visiting cradles, handlines strung the length of it. | sailing board, ports, handlines, inspection slots, boarding collars |
| `room.l02_pressure` | L02 Pressure Lobby | Hub | indoor | public | — | A round weightless chamber between two pressure doors, the spoke ladder's well at its centre. | placard, spoke well, small placard, cycle counter |
| `room.l03_arrivals` | L03 Orbital Access Arrivals | Drum (1) | indoor | public | `arrivals` | The first room in spin, padded and railed for people who have just remembered they have weight. | placards, viewport strip, benches, ladder foot |
| `room.l04_concourse` | L04 Drum Concourse | Drum | circulation | public | — | The drum's one wide hall, its floor curving up ahead and behind, hydroponic racks down the middle. | hydroponic racks, conservation posters, viewports, notices, floor |
| `room.l05_registry` | L05 Registry Annex | Drum | indoor | public | `registry` | A quiet panelled annex of numbered windows, the Registry's seal above the counter. | seal, permit press, registers, brass plate |
| `room.l06_clearing` | L06 Clearing House Window | Drum | indoor | public | `clearing` | A long glass window onto racked clearing machines, a counter and a slot on the near side. | machines, brass plate, board, slot |
| `room.l07_exchange` | L07 Cabeus Exchange | Drum | indoor | public | `exchange` | A broad spun trading room, the Cabeus and Crisium asks on one wall and the receiving bids on the other. | ask board, sample billets, open crate, permit label, stair |
| `room.l08_receiving` | L08 Habitat Board Receiving | Hub | indoor | public via the L07 stair or the dockmaster's inner door | `receiving` | A long weightless freight hall where the Habitat Board's landers mate to the hub. | intake board, freight collar, assay rig, service hatch, stair |
| `room.l09_chandlery` | L09 Victualling Office | Drum | indoor | public | `chandlery` | A stores counter with ration crates racked behind it and a bunkering panel beside the till. | board, bunkering panel, crates, fill card |
| `room.l10_cold_trap` | L10 The Cold Trap | Drum | indoor | public | — (the social room) | A blue-lit bar on the drum's curve, named for the shadowed craters at the pole. | bar, corner table, noticeboard, pour, tumblers |
| `room.l12_dockmaster` | L12 Dockmaster's Office | Hub | indoor | public | `dockmaster` | A cramped weightless office with the sailing board on its back wall and the yards' slips in its window. | sailing board, window, clock, passage card |
| `room.l15_spine` | L15 Rim Service Spine | Hub | indoor | restricted | `spine` | A weightless service trunk of cable trays and storefront cages, a glassed office at its inner end. | cages, office, bearings |

Every desc runs 101–118 words. The IC return lands a pilot in L03 (`ic_return::PORTS`,
`tacsim/src/ic_return.rs:76`); L03 is the first room in weight, and its `ladder foot` says WEIGHT RETURNS
HERE. Ceres's Office Deck has no counterpart: the principals sit in the spine, on the Stickney pattern.

**Listings** (lane v8's `luna_port`; every vendor's agent is the port's clerk, `luna_port.clerk`; the
boards are the role words above):

| Org (`id`, capital) | Signatory | Storefront (in L15) | Stock | Asks | Bids |
|---|---|---|---|---|---|
| Concord Orbital Access (`orbital_access`, 50,000cr) | — (no vendor: the port authority) | — | — | — | — |
| Cabeus Works (`cabeus_works`, 250,000cr) | Lorcan Mwangi-Eriksen | `item.cabeus_cage` "Cabeus Works stock cage" | structural alloy stock 3,120 t | structural alloy stock 400cr/t (exchange) | metal concentrate 270cr/t, intake 800 t (receiving, exchange) |
| Lunar Habitat Board (`habitat_board`, 1,000,000cr) | Thandiwe Albrecht-Sato | `item.habitat_manifold` "Habitat Board intake manifold" | — (bid-only) | — | water ice 195cr/t, intake 4,100 t; carbon billet 380cr/t, intake 300 t (receiving, exchange) |
| Crisium Instruments (`crisium_instruments`, 50,000cr) | Anneliese Okafor-Thistlewood | `item.crisium_cage` "Crisium bonded cage" | instrument boards 2,080 t | instrument boards 900cr/t (exchange, clearing) | — |
| Highport Victualling Office (`highport_victualling`, 50,000cr) | Imogen Batista-Okeke | `item.victualling_locker` "Victualling Office stores locker" | reaction mass 5,000 t; provisions 2,600 crew-days | reaction mass 60cr/t (chandlery, dockmaster); provisions 10cr a crew-day (chandlery) | — |

Crisium carries the port's emergency block (provisions, water ice, reaction mass; ratio 1,500 ‰, intake
100); the premium is `{300, 500}`; routes `deimos` → `aitken_voltaire`, `titan` → `aitken_ligeia`. The
room text carries none of these figures: the boards and the counters name the lines and say the figures
are the houses' own.

### 5.4 The cast

| entry | Name (as authored) | Room | House / office | Interfaces | Tier | Kind | Knows (the pack's `knows`, condensed) |
|---|---|---|---|---|---|---|---|
| `npc.mwangi_eriksen` | Lorcan Mwangi-Eriksen | L07 | Sales Agent, Cabeus Works; **signatory** of the alloy ask and the concentrate bid | market, atlas | minor | cast | the alloy ask and the concentrate bid and what remains; the aluminium–titanium regolith alloy, the dense partner of the boards; sold toward Deimos and Titan, never Ceres; finite, never topped up |
| `npc.albrecht_sato` | Thandiwe Albrecht-Sato | L08 | Receiving Commissioner, Lunar Habitat Board; **signatory** of the ice and billet bids | market, atlas | minor | cast | the Board's bids, intake and funds; the loops lose carbon and Titan billet tops them up; the landers take the bulk down; Cabeus shares the receiving board |
| `npc.okafor_thistlewood` | Anneliese Okafor-Thistlewood | L07 | Export Factor, Crisium Instruments; **signatory** of the boards ask | market, help | minor | cast | the boards ask and the stock; crated from the chartered Earth-basin fabs, they cube out before they weigh out; every crate under a Registry export permit; nothing at Ceres bids a board; Deimos and Titan buy |
| `npc.batista_okeke` | Imogen Batista-Okeke | L09 | Purser, Highport Victualling Office; **signatory** of the reaction mass and provisions asks | market | extra | cast | the posted prices and stock; reaction mass metered to the cradle; two aboard need twice the days; Deimos a week or two and Titan nearer two months in a good season, single-handed |
| `npc.kowalczyk_amadi` | Dockmaster Aurelio Kowalczyk-Amadi | L12 | Dockmaster, Concord Orbital Access | atlas, travel | minor | cast | the cradles; a Dray files for Deimos or Titan and nowhere else; Titan only throttled, `throttle=450`, one soul, on every listed Titan leg; the light lag to Titan; the layout |
| `npc.holt_iversen` | Deputy Registrar Kwabena Holt-Iversen | L05 | Deputy Registrar, the Registry's highport annex | atlas, help | extra | cast | the Registry founded at Luna about 2140, older than the Concord, mirrored at Ceres; export permits are Registry paper; it records and notarises, never grants; the Saturn Charter of 2291 and the Townships' petitions are recorded here |
| `npc.devereux_anand` | Mireille Devereux-Anand | L06 | Clearing Clerk, Concord currency clearing | help | extra | cast | clearing as a pooled function; every credit clears through Luna, one currency; Ceres Clearing and the Republic both owe Luna; the hours |
| `npc.ferro_wainwright` | Maren Ferro-Wainwright | L05 | Registry Export Clerk, the highport annex | help | extra | cast | the permit bound to each crate's manifest; the press counts its strokes; the annex grants nothing |
| `npc.rasmussen_oto` | Ngozi Rasmussen-Oto | L10 | Keeper of the Cold Trap | none | extra | cast (tags add `tycho`) | the bar's name; Tycho's underwriters at the corner table; the Terminal's prices stale by light; the spoke's saying |
| `npc.ferreira_lund` | Cosima Ferreira-Lund | L15 | Director, Concord Orbital Access at Aitken Highport | none | major | principal, `enabled:false` | not voiced in the slice |
| `npc.kwarteng_fairweather` | Odile Kwarteng-Fairweather | L15 | Chair, Lunar Habitat Board | none | major | principal, `enabled:false` | not voiced in the slice |
| `npc.rourke_haddad` | Benedikt Rourke-Haddad | L15 | Managing Director, Crisium Instruments | none | major | principal, `enabled:false` | not voiced in the slice |
| `npc.moravec_ansah` | Ingrid Moravec-Ansah | L15 | Works Director, Cabeus Works | none | major | principal, `enabled:false` | not voiced in the slice |

Every cast NPC carries a `persona`, a `speech_style`, `does_not_know` bounds and a non-LLM `fallback`
list, and the tags `corridor_trade` and `port_aitken`. Officials carry their title first in `name` (the
Stickney pattern; `NpcDef` has no title field). No doc 11 principal is placed here.

### 5.5 Lore pool (`lore.port_aitken`, tag `port_aitken`; the cast may draw `corridor_trade`, and the keeper `tycho`)

*Public* (ten): The pocket · Aitken Highport · The Belt's door · Why not home to Ceres · The Registry ·
Credits clear through Luna · The yards · Carbon · Malapert · The spoke.

*Guarded* (three): The Board's ice · The rotation · Tycho and Titan.

*Secret* (one): `secret.half_afloat`, `reveal: staff`, about `npc.kwarteng_fairweather`. Stored as a
Secret; its truth is in the pack and in no prompt and not in this brief.

### 5.6 Ambience

Twelve lines (the validator's cap is 24): the drum's bearing note as a lander mates, Earth past the
viewports, slow falls and slow pours in the spin, the grow lamps' dusk, the sailing board turning over,
the polar Sun across the hub ports, the clearing lamps, the Cold Trap carrying along the curve, the
conservation notice's carbon tally, and Shackleton's lamps under the hub with the yards' hooded slips after
them.

### 5.7 Geometry, exits and walls

Neighborhood footprint [[100,80],[220,200]], site and structure [[110,90],[210,190]]; budget target 12,
maximum 16. **Hub Deck** (floor 0): L01 [110,100]–[210,115]; L02 [110,115]–[140,140]; L12
[140,115]–[185,140]; L08 [140,140]–[210,165]; L15 [140,165]–[185,190] (restricted, under L07). **Drum
Deck** (floor 1, z 6): L03 [110,115]–[140,140]; L05 [110,140]–[140,160]; L06 [110,160]–[140,190]; L04
[140,115]–[185,160] (circulation); L07 [140,160]–[185,190]; L09 [185,115]–[210,160]; L10
[185,160]–[210,190]. The Drum Deck's horizontal split sits at y 160, not 165, so the stair's foot lies in
L08 and outside the spine. **Cross-deck:** L02↔L03, the spoke, `mode: ladder`, `portal.at`
(125,128,0)–(125,128,6), `dz_m` +6; L07↔L08 public stair, `portal.at` (165,162,6)–(165,162,0), `dz_m` −6;
L08→L15 hatch on their shared edge (y 165, x 140–185), `lock: staff`, `visibility: hidden`. **Walls:** one
— L01|L12 (y 115, x 140–185, z 0–6), the Hub Deck's one exitless public seam; the Drum Deck has none (L09|L07
and L04|L10 meet at a corner only). **Counters** (features, `room` set): L08 (145,160)–(175,160), L12
(150,135)–(175,135), L05 (112,150)–(138,150), L06 (112,175)–(138,175), L09 (205,120)–(205,155).
Topology: the gallery's `port.dock` seed → L02 → L12 → L08 → the stair → L07 → L04 → L03, L05, L09, L10
→ L06.

| entry | From → to | Mode | Way in (`names[0]`) | Way back (`names[1]`) |
|---|---|---|---|---|
| `exit.l01_l02` | L01 → L02 | door | `Pressure Lobby <PL>;pl;lobby;in` | `Berth Gallery <BG>;bg;gallery;berths;out` |
| `exit.l02_l12` | L02 → L12 | door | `Dockmaster <DM>;dm;dockmaster;office` | `Pressure Lobby <PL>;pl;lobby;out` |
| `exit.l12_l08` | L12 → L08 | door | `Receiving <RC>;rc;receiving;habitat` | `Dockmaster <DM>;dm;dockmaster;out` |
| `exit.l02_l03` | L02 → L03 | ladder (the spoke), dz +6 | `Spoke Ladder <SL>;sl;spoke;ladder;drum` | `Spoke Ladder <SL>;sl;spoke;ladder;hub` |
| `exit.l03_l04` | L03 → L04 | door | `Concourse <C>;c;concourse;drum;in` | `Arrivals <AR>;ar;arrivals;access` |
| `exit.l03_l05` | L03 → L05 | door | `Registry Annex <RA>;ra;registry;annex` | `Arrivals <AR>;ar;arrivals` |
| `exit.l05_l04` | L05 → L04 | door | `Concourse <C>;c;concourse;out` | `Registry Annex <RA>;ra;registry;annex` |
| `exit.l05_l06` | L05 → L06 | door | `Clearing House <CL>;cl;clearing;window` | `Registry Annex <RA>;ra;registry;annex` |
| `exit.l06_l07` | L06 → L07 | door | `Exchange <X>;x;exchange;cabeus;market` | `Clearing House <CL>;cl;clearing;window` |
| `exit.l04_l07` | L04 → L07 | door | `Exchange <X>;x;exchange;cabeus;market` | `Concourse <C>;c;concourse;out` |
| `exit.l04_l09` | L04 → L09 | door | `Victualling <VO>;vo;victualling;chandlery;stores` | `Concourse <C>;c;concourse;out` |
| `exit.l07_l10` | L07 → L10 | door | `Cold Trap <CT>;ct;trap;commons;bar` | `Exchange <X>;x;exchange;out` |
| `exit.l09_l10` | L09 → L10 | door | `Cold Trap <CT>;ct;trap;commons;bar` | `Victualling <VO>;vo;victualling;stores` |
| `exit.l07_l08` | L07 → L08 | stair, dz −6 | `Receiving Stair <RS>;rs;receiving;habitat;down;d` | `Exchange Stair <ES>;es;exchange;up;u` |
| `exit.l08_l15` | L08 → L15 | hatch, hidden, `lock: staff` | `Service Spine <SS>;ss;spine;hatch` | `Receiving Hatch <RH>;rh;receiving;out` |

Fifteen exits; `exit.l08_l15` is the locked one, and a paired exit locks only the way in.

---

## 6. Voltaire Anchorage, Deimos

`region.voltaire_anchorage` (surface region "Deimos", body `deimos`; locality
`locality.voltaire_anchorage`, kind `port`) + `neighborhood.voltaire_anchorage` v1 (requires
`region.voltaire_anchorage`@1); site `site.voltaire_roads` "Voltaire Roads" (`site.transit`) and structure
`structure.moorings_block`, Tank Deck index 0 (z 0 m, 7 m) and Roads Deck index 1 (z 7 m, 4.5 m). Eleven
rooms, two decks, no Limbo door: reachable only by flight. Lane v8's `deimos_port` (location
`voltaire_anchorage`, venue `deimos`) names it; the boxes are Stickney's, deliberately. Every name below
awaits the owner's "names ok" (§4).

### 6.1 Theme (`neighborhood.voltaire_anchorage`; the 2350 base on `region.voltaire_anchorage`)

**Name.** TacSim 2350: the Roads.

**Style guide** (the pack's, condensed). Present tense, no second person, no named NPCs in a room desc;
each room distinct in sound, light and smell before furniture. Deimos holds even less than Phobos (doc 10
§3): hulls settle into cradles and are tied down, decks are painted conventions, handlines run everywhere,
benches have tether loops. The Roads are the Republic at its border — institutional, bonded and sealed,
forms in triplicate, the Commissioner's seal on every manifest that goes on to Phobos. The arks are
everywhere underfoot: salvaged plating with hull names on it, ark-era scrubbers, a galley turned bar, and
the *Long Mercy* over the crater. The Reserve is in the floor and says little about itself; its rotation
stamps and its posted prices are all it shows. Mars is a small rust disc in the sky. Desc length medium
(60–120 words); details one or two dry sentences with a fact in them.

**Vocabulary.** roads, anchorage, cradle, tie-down, handline, tether loop, pressure door, inspection
sticker, bond, in bond, bonded, seal, Commissioner's seal, customs post, roadsmaster, sailing plan,
clearance, Reserve, rotation, warden, tank gallery, ark, hulk, moorings, Long Mercy, breakers, Voltaire,
Deimos, Phobos, Stickney, Syrtis, Coprates, Kasei, the Guard, Vigil, the hop, light alloy, instrument
boards, carbon billet, water ice, reaction mass, crew-day, tonne, Republic.

**Banned.** warp, hyperspace, FTL, teleport, alien, android, gravity plating, shields, stardate, hangar
bay, you see.

### 6.2 Overview

Voltaire Anchorage is the Republic's outer anchorage, dug into the floor of Voltaire crater on Deimos,
where the Enclave arks lay in the roads from the 2150s to the 2200s and their contract crews stayed to
break the hulls up for the first Martian habitats. A Dray comes in on a long throttled fall from Titan or a
shorter one from Luna and settles; it leaves by the one lane the Roads post, the hop down to Phobos, about
three hours and a tonne of reaction mass in a good season. The berths are cradles with tie-downs, because
on Deimos a hull resting on the crater floor would walk off it under the push of its own pumps. Over the
crater rests the last ark hulk, the *Long Mercy*, kept as a monument; beyond it Mars is a rust disc less
than half the size it shows from Phobos. Under the floor lies the Republic Strategic Reserve — reaction
mass, water and fixed nitrogen against the day the Belt stops selling. The Concord fuelled its 2308
blockade of Mars from a depot here and conveyed it to the Republic with the debt in 2312, as part of the
Settlement. The air is old ark-era scrubber air with cold tank steel and new tether rubber in it, and the
commissary gives off the cardboard-and-salt smell of rotated rations.

The station is laid out as Stickney is: the visiting berths and the pressure lobby at one end; the
Republic Customs Post the first desk every arrival stands at; the Roads Concourse the hall every public
room opens off — the Roadsmaster's Office, the Bond Exchange (its public stair down to the Reserve
Receiving Hall on the Tank Deck), the Reserve Commissary, the Long Straw and the Ark Moorings Gallery,
whose ports look out on the *Long Mercy*. One restricted room, the Reserve Spine, is staff-locked and
hidden off the receiving hall; it holds the storefront cages and the dark desks of the Reserve Warden, the
Bond Commissioner and Syrtis's managing director.

Its trade is the bond. Concord and Company goods enter Mars here, in bond, and leave bond only by the hop
to Stickney's Customs Hall under the Commissioner's seal; Belt cargo clears at Stickney, as it always has,
and the Republic does not bond its own yard stock, so no lane to the Roads is posted from Phobos. The
anchorage sells nothing on its exchange floor: Syrtis Bonded Imports bids Luna's light alloy and crated
boards — Kasei rolls ferrous stock from Belt concentrate, and the aluminium–titanium alloy the Yard's
frames and pressure hulls want comes from the lunar regolith lines and nowhere on Mars — and the Reserve
bids Titan's ice and carbon billet and sells its bunkers at the Republic's posted price, the same as
Kasei's. The ice bid sits under Kasei's by design, and the receiving board says so in pencil. Hulls
arrive from Luna and Titan. The governance is the Republic's, de jure and de facto: the Voltaire Bond
Authority (an org with no vendor) under the Republic's customs service, the Reserve's wardens, the
Orbital Guard's Vigils exercising off the Roads. Every org carries the faction `republic`.

### 6.3 The rooms

| id | Room | Deck | Kind | Access | Port role | Short (the pack's `short_desc`) | Details (`look` keys) |
|---|---|---|---|---|---|---|---|
| `room.d01_berth_gallery` | D01 Visiting Berth Gallery | Roads (1) | indoor | public | gallery; `port.dock` | A long pressurised gallery along the visiting cradles, every hull beyond the ports lashed down by its tie-downs. | berth board, tie downs, ports, handlines |
| `room.d02_pressure` | D02 Pressure Lobby | Roads | indoor | public | — | A square chamber between two pressure doors, breathing through an ark-era scrubber. | placard, scrubber stack, cycle counter |
| `room.d03_customs` | D03 Republic Customs Post | Roads | indoor | public | `customs` | A narrow customs post of two queue rails and a counter, an origin board behind it under the Republic's crest. | origin board, forms, seal press, crest |
| `room.d04_concourse` | D04 Roads Concourse | Roads | circulation | public | — | The broad hall of the Roads Deck, its walls plated with salvage from broken arks. | ark plating, station map, handlines, patrol notice, clock |
| `room.d05_harbourmaster` | D05 Roadsmaster's Office | Roads | indoor | public | `harbourmaster` | A cramped office behind a half-door, the roads board glowing on its back wall. | roads board, sailing plan pad, clocks, hop card, radio |
| `room.d06_moorings` | D06 Ark Moorings Gallery | Roads | indoor | public | — (the *Long Mercy* in the ports) | A dim gallery of tall ports looking out over the crater at the last ark hulk. | Long Mercy, plaque, Mars, benches |
| `room.d07_receiving` | D07 Reserve Receiving Hall | Tank (0) | indoor | public via the D08 stair | `receiving` | A receiving counter on the Tank Deck before a window onto the Reserve's tank gallery. | intake board, assay scales, tank window, hatch |
| `room.d08_exchange` | D08 Bond Exchange | Roads | indoor | public | `exchange` | A low trading room with one board of bonded buyers' bids and no ask side at all. | bids board, blank panel, pencil note, inspection crate, stair well |
| `room.d09_commissary` | D09 Reserve Commissary | Roads | indoor | public | `commissary` | A stores counter selling the Reserve's rotated rations, every crate stamped with two dates. | price posting, rotation stamps, crates, cold locker |
| `room.d10_long_straw` | D10 The Long Straw | Roads | indoor | public | — (the social room) | A bar built into a salvaged ark galley, the Titan card pinned by its hatch. | galley, name board, roads chart, Titan card, chalk list |
| `room.d15_spine` | D15 Reserve Spine | Tank | indoor | restricted | `spine` | A service corridor behind the tank gallery, two storefront cages and a glassed office at its end. | cages, office, gauges, manifolds |

Every desc runs 97–117 words. The IC return lands a pilot in D01, the gallery (`ic_return::PORTS`). The
`sailing plan pad` in D05 carries a bond seal number, "the line Stickney's pad does not have"; the `hop
card` beside it gives the hop's three hours and its tonne in a good season.

**Listings** (lane v8's `deimos_port`; every vendor's agent is `deimos_port.clerk`):

| Org (`id`, capital) | Signatory | Storefront (in D15) | Stock | Asks | Bids |
|---|---|---|---|---|---|
| Voltaire Bond Authority (`voltaire_bond`, 50,000cr) | — (no vendor: the operator) | — | — | — | — |
| Republic Strategic Reserve (`strategic_reserve`, 300,000cr) | Caspian Abiodun-Fleck | `item.reserve_manifold` "Reserve intake manifold" | reaction mass 5,000 t; provisions 2,600 crew-days | reaction mass 60cr/t (commissary, harbourmaster); provisions 10cr a crew-day (commissary) | water ice 190cr/t, intake 900 t; carbon billet 380cr/t, intake 300 t (receiving) |
| Syrtis Bonded Imports (`syrtis_bonded`, 3,100,000cr) | Emeka Strand-Villanueva | `item.syrtis_cage` "Syrtis bonded cage" | — (bid-only) | — | structural alloy stock 460cr/t, intake 2,400 t; instrument boards 1,100cr/t, intake 1,600 t (receiving, exchange) |

Syrtis carries the port's emergency block (provisions, water ice, reaction mass; ratio 1,500 ‰, intake
100); the premium is `{300, 500}`; one route, `stickney` → `voltaire_stickney`. The goods table does not
grade alloys, so the "light alloy stock, Concord origin" of the bids board is `structural_alloys` in the
lane; the distinction is lore (A.8 design Q5). The exchange's `blank panel` reads NO SALES IN BOND.

### 6.4 The cast

| entry | Name (as authored) | Room | House / office | Interfaces | Tier | Kind | Knows (the pack's `knows`, condensed) |
|---|---|---|---|---|---|---|---|
| `npc.abiodun_fleck` | Caspian Abiodun-Fleck | D07 | Quartermaster, Republic Strategic Reserve; **signatory** of the rotation sales (reaction mass, provisions) and the ice and billet bids | market, atlas | minor | cast | the Reserve's lines, intake and stock; the ice bid under Kasei's by design; billet from Titan for Mars's soil; reaction mass at Kasei's price; rotation sales; both decks |
| `npc.strand_villanueva` | Emeka Strand-Villanueva | D08 | Bonded Buyer, Syrtis Bonded Imports; **signatory** of the alloy and boards bids | market, help | minor | cast | Syrtis's bids and intake; Kasei's ferrous line against the lunar light alloy the Yard frames with; the premium Coprates calls tribute; boards cube out; bond left only by the hop under the seal; nothing sold on the floor |
| `npc.rautio_grant` | Femi Rautio-Grant | D03 | Customs Inspector, the Republic's customs service, under the Voltaire Bond Authority | atlas, travel, help | minor | cast | Concord and Company goods into bond, Belt cargo cleared at Stickney; no lane posted from Phobos; the layout; the hop's three hours; how to file with the roadsmaster |
| `npc.szabo_mensah` | Aurek Szabo-Mensah | D05 | Roadsmaster, Voltaire Bond Authority | atlas, travel | extra | cast | the cradles, in bond or cleared for Stickney; settle and tie down; the hop and the one lane out; the falls from Luna and Titan; sailing plans and IDLE; the ark moorings |
| `npc.kettunen_ade` | Marisol Kettunen-Ade | D10 | Keeper of the Long Straw | none | extra | cast | the ark moorings as the Roads tell them; the *Long Mercy* a monument, not a wreck; the bar's name; who is in port; the crew cards |
| `npc.tsai_barros` | Wilhelmina Tsai-Barros | D10 | Agent of the Titan Concession Company at the Roads | help | extra | cast | Ligeia Terminal; the Saturn Charter of 2291; the Townships' petitions; what Titan sells; Titan reached only throttled, by the lane from Luna; billet for the Reserve |
| `npc.delgado_mertens` | Radmila Delgado-Mertens | D15 | Reserve Warden, Republic Strategic Reserve | none | major | principal, `enabled:false` | not voiced in the slice |
| `npc.bergstrom_nwachukwu` | Anders Bergström-Nwachukwu | D15 | Bond Commissioner, Voltaire Bond Authority | none | major | principal, `enabled:false` | not voiced in the slice |
| `npc.okwuosa_brandao` | Liesel Okwuosa-Brandão | D15 | Managing Director, Syrtis Bonded Imports | none | major | principal, `enabled:false` | not voiced in the slice |

Every cast NPC carries a `persona`, a `speech_style`, `does_not_know` bounds and a non-LLM `fallback`
list, and the tags `corridor_trade` and `port_voltaire`. No doc 11 principal is placed here; the pool's
"The Guard off the Roads" names Commodore Bask's Kirkwood pickets as planned from Phobos.

### 6.5 Lore pool (`lore.port_voltaire`, tag `port_voltaire`; the cast may draw `corridor_trade`)

*Public* (ten): The Ark Moorings · The blockade depot · The conveyance · The long straw · The bond · Lunar
light alloy · The Reserve's ice · Bunkers at the Roads · The Guard off the Roads · The Company's agent.

*Guarded* (two): Rotation sales · The treaty inspector.

*Secret* (one): `secret.reserve_level`, `reveal: staff`, about `npc.delgado_mertens`. Stored as a
Secret; its truth is in the pack and in no prompt and not in this brief.

### 6.6 Ambience

Twelve lines: a pressure door cycling toward the cradles, a hull settling so slowly the tie-down crew
finishes its coffee, the ark-era scrubbers' cough, a Reserve pump's count of ten, a Vigil's callsign and
an exercise box on the approach channel, the *Long Mercy*'s cables taking up the cold, the Titan card in
the Long Straw, a dolly of rotated ration crates, the seal press carried across the concourse, Syrtis
coming into the light on Mars, the berth board going from IN BOND to CLEARED FOR STICKNEY, and a school
group clipped in a line on its way to the moorings gallery.

### 6.7 Geometry, exits and walls

Neighborhood footprint [[100,80],[210,190]], site and structure [[110,90],[200,180]] — Stickney's;
budget target 11, maximum 16. **Roads Deck** (floor 1, z 7): D01 [110,100]–[200,115]; D02
[110,115]–[125,130]; D03 [110,130]–[125,155]; D04 [125,115]–[185,155] (circulation); D06
[185,115]–[200,155]; D05 [110,155]–[132,180]; D08 [132,155]–[155,180]; D09 [155,155]–[177,180]; D10
[177,155]–[200,180]. **Tank Deck** (floor 0): D07 [110,150]–[140,180]; D15 [140,150]–[170,180]
(restricted, under D04's south edge, D08 and D09). **Cross-deck:** D08↔D07 public stair, `portal.at` (136,167,7)–(136,167,0),
`dz_m` −7 (inside both rooms, Stickney's P08↔P07 point moved two metres); D07→D15 hatch on their shared
edge (x 140, y 150–180), `lock: staff`, `visibility: hidden`. **Walls**, all seven exitless public seams:
D01|D04 (y 115, x 125–185), D01|D06 (y 115, x 185–200), D02|D04 (x 125, y 115–130), D03|D05 (y 155,
x 110–125), D05|D08 (x 132), D08|D09 (x 155), D09|D10 (x 177), each z 0–4.5. **Counters:** D03
(112,153)–(123,153), D05 (112,157)–(130,157), D07 (112,177)–(138,177) on the Tank Deck, D09
(157,176)–(175,176). Topology: the gallery's `port.dock` seed → D02 → D03 → D04 → D05, D06, D08, D09, D10;
D08 → the stair → D07.

| entry | From → to | Mode | Way in (`names[0]`) | Way back (`names[1]`) |
|---|---|---|---|---|
| `exit.d01_d02` | D01 → D02 | door | `Pressure Lobby <PL>;pl;lobby;in` | `Berth Gallery <BG>;bg;gallery;berths;out` |
| `exit.d02_d03` | D02 → D03 | door | `Customs Post <CP>;cp;customs;in` | `Pressure Lobby <PL>;pl;lobby;out` |
| `exit.d03_d04` | D03 → D04 | door | `Concourse <C>;c;concourse;roads;in` | `Customs Post <CP>;cp;customs;locks` |
| `exit.d04_d05` | D04 → D05 | door | `Roadsmaster <RM>;rm;roadsmaster;harbourmaster;harbour` | `Concourse <C>;c;concourse;out` |
| `exit.d04_d08` | D04 → D08 | door | `Bond Exchange <X>;x;exchange;bond;market` | `Concourse <C>;c;concourse;out` |
| `exit.d04_d09` | D04 → D09 | door | `Commissary <CM>;cm;commissary;stores` | `Concourse <C>;c;concourse;out` |
| `exit.d04_d10` | D04 → D10 | door | `Long Straw <LS>;ls;straw;commons;bar` | `Concourse <C>;c;concourse;out` |
| `exit.d04_d06` | D04 → D06 | door | `Moorings Gallery <MG>;mg;moorings;arks;windows` | `Concourse <C>;c;concourse;out` |
| `exit.d06_d10` | D06 → D10 | door | `Long Straw <LS>;ls;straw;commons;bar` | `Moorings Gallery <MG>;mg;moorings;arks` |
| `exit.d08_d07` | D08 → D07 | stair, dz −7 | `Receiving Stair <RS>;rs;receiving;reserve;down;d` | `Exchange Stair <ES>;es;exchange;up;u` |
| `exit.d07_d15` | D07 → D15 | hatch, hidden, `lock: staff` | `Reserve Spine <SS>;ss;spine;hatch` | `Receiving Hatch <RH>;rh;receiving;out` |

Eleven exits; `exit.d07_d15` is the locked one.

---

## 7. Ligeia Terminal, Titan

`region.ligeia_terminal` (surface region "Titan", body `titan`; locality `locality.ligeia_terminal`, kind
`port`) + `neighborhood.ligeia_terminal` v1 (requires `region.ligeia_terminal`@1); site
`site.ligeia_spar` "Ligeia Spar" (`site.transit`) and structure `structure.ligeia_spar`, Cargo Deck index
0 (z 0 m, 7 m) and Company Deck index 1 (z 7 m, 4.5 m). Twelve rooms, two decks, no Limbo door: reachable
only by flight, and only throttled. Lane v8's `titan_port` (location `ligeia_terminal`, venue `titan`)
names it. Every name below awaits the owner's "names ok" (§4).

### 7.1 Theme (`neighborhood.ligeia_terminal`; the 2350 base on `region.ligeia_terminal`)

**Name.** TacSim 2350: the Terminal.

**Style guide** (the pack's, condensed). Present tense, no second person, no named NPCs in a room desc;
each room distinct in sound, light and smell before furniture. The spar is unspun and weightless (no
artificial gravity beyond thrust and rotation): decks are painted conventions, handlines run everywhere,
benches have tether loops, everything loose is netted or corded. It is cold: frost on the pressure seals,
breath showing near the berths, heaters ticking. The air smells of naphtha and ammonia the scrubbers never
quite take out, and of bitter almond near the billet locks. Saturn is huge and banded in the long window,
the Sun a hard white star, Titan a featureless orange ball until a lifter's plume punches through. The
Company sounds like a company town — orange lettering, posted notices, the Company's book, everything at
the Company's price; the townships sound plain — hand-painted placards, their own heaters, petitions under
film. News from Luna is always over an hour old. Desc length medium (60–120 words); details one or two dry
sentences with a fact in them.

**Vocabulary.** spar, Terminal, the Company, the Company's price, the Company's book, Chief Factor,
Charter, Saturn Charter, townships, Selk, the Board, petition, Shackleton, lifter, lifter window, Ligeia,
haze, overburden, carbon billet, billet lock, water ice, reaction mass, ninety-four kelvin, handline,
tether loop, frost, pressure seal, collar, berth, clearance, sailing plan, light lag, Luna's board,
throttled, single-handed, crew-day, tonne.

**Banned.** warp, hyperspace, FTL, teleport, alien, android, gravity plating, shields, stardate, hangar
bay, you see.

### 7.2 Overview

Ligeia Terminal is the Titan Concession Company's orbital terminal, a long unspun spar in low Titan orbit,
named for the methane sea the Company's lifters climb from. The lifters come up out of the orange haze to
it with carbon billet and the overburden ice and take the inbound down; the spar crosses the sea for a run
of orbits each Titan day and then not for a fortnight, never standing over it. No Dray could leave Titan's
surface at its thrust, and few reach the spar except slowly: the dockmaster's board reads COME SLOW OR
DON'T COME, because the passage from Luna, or home to Luna or Mars, fits an 80-tonne tank only at half
throttle — `throttle=450`, one soul aboard, on every listed leg — and most arrivals are single-handed; from Luna it
is nearer two months in a good season. The spar is weightless and cold, frost on every seal, naphtha and
ammonia in the air and bitter almond near the billet locks, which the Company's notices call harmless at
the levels shown. Saturn hangs huge and banded in the long window, three fingers' width at arm's length; below, Titan is an orange ball until a lifter's plume
punches through.

The Terminal is laid out along the spar in two decks. The visiting berths cross the spar's head on the
Company Deck; the pressure lobby opens on Company Arrivals and the Spar Concourse, and every public desk
opens off the concourse — the dockmaster, the Company Sales Floor (its public stair down to Townships
Receiving on the Cargo Deck), the Ninety-Four and the Lifter Gallery under the long window — with the
Company Store reached from arrivals and from the bar. On the Cargo Deck the townships' counter, the one the
Company does not own, opens on the Bunkering Yard, and a staff-locked hidden hatch off the yard leads to
the Spar Service Spine, which holds the storefront cages and two offices facing each other across the
corridor: the Chief Factor's and the Speaker's.

It is a company town under a charter. The Concord's Saturn Charter of 2291, recorded at the Registry, a
ninety-nine-year grant lapsing in 2390, makes the Titan Concession Company proprietor and government of
the Saturn system, headed by its Chief Factor, clearing through Luna and months from anyone who could
overrule it. The Selk Townships Board is the opposition: Selk, the first township, petitions Shackleton for
a Concord charter of its own, the petitions lodged at the Registry, and the Board's office sits on the
Company's service deck by the Charter. The
Company sells billet, the overburden ice at its handling cost, the cheapest reaction mass in the system
and the dearest stores, all at the Company's price; the Board buys alloy stock and instrument boards at
frontier prices because everything breaks at ninety-four kelvin. There is no relay half, so news comes by
light, over an hour stale from Luna and up to an hour and a half. Hulls arrive from Luna; they leave for
Luna or for Deimos. The Company's two houses carry the faction `company`, the Board `townships` — the label
is an org's allegiance, not its port's polity. The risk zone is frontier.

### 7.3 The rooms

| id | Room | Deck | Kind | Access | Port role | Short (the pack's `short_desc`) | Details (`look` keys) |
|---|---|---|---|---|---|---|---|
| `room.t01_berth_gallery` | T01 Visiting Berth Gallery | Company (1) | indoor | public | gallery; `port.dock` | A long cold gallery across the head of the spar, handlines strung the length of it and frost on every seal. | berth board, ports, handlines, frost |
| `room.t02_pressure` | T02 Pressure Lobby | Company | indoor | public | — | A square chamber of pressure doors between the berths and the spar, nothing in it but tether clips. | placard, cycle counter, inspection tag |
| `room.t03_arrivals` | T03 Company Arrivals | Company | indoor | public | `arrivals` | A low hall of queue rails ending at a single Company desk under the posted Charter notice. | charter notice, forms, lifter board, desk |
| `room.t04_concourse` | T04 Spar Concourse | Company | circulation | public | — | The spar's widest corridor, handlines both ways and Company notices pinned flat under film. | notices, station map, handlines, fans |
| `room.t05_dockmaster` | T05 Dockmaster's Office | Company | indoor | public | `dockmaster` | A narrow office behind a half-door, its back wall a hand-lettered board that reads COME SLOW OR DON'T COME. | motto board, slow board, clock, sailing plan pad, radio |
| `room.t06_lifter` | T06 Lifter Gallery | Company | indoor | public | — (the long window) | A dark gallery along the long window, Saturn huge and banded above and Titan orange below. | Saturn, Titan, lifter plume, window, Sun |
| `room.t07_exchange` | T07 Company Sales Floor | Company | indoor | public | `exchange` | A bare trading floor with the Company's prices on the long wall and a stair well down to receiving. | price board, townships board, Luna board, stair well, almond notice |
| `room.t08_receiving` | T08 Townships Receiving | Cargo (0) | indoor | public via the T07 stair | `receiving` | A scarred bench at the foot of the stair, the one counter on the spar the Company does not own. | intake placard, mass gauge, lockers, heaters, petition notice |
| `room.t09_commissary` | T09 Company Store | Company | indoor | public | `commissary` | A stores counter under the Company's posting, crew-day crates netted to the racks behind. | price posting, crates, ledger, cage |
| `room.t10_ninety_four` | T10 The Ninety-Four | Company | indoor | public | — (the social room) | A padded low room with a hatch bar, a huge thermometer and crews keeping to their own sides. | thermometer, notice wall, bulbs, sides, name |
| `room.t11_bunkering` | T11 Bunkering Yard | Cargo | indoor | public via T08 | `yard` | A frosted tank gallery of lagged transfer lines, thudding pumps and the billet locks at its far end. | transfer lines, billet locks, pumps, hatch |
| `room.t15_spine` | T15 Spar Service Spine | Cargo | indoor | restricted | `spine` | A service corridor of cable trays and storefront cages, two offices facing each other at its end. | cages, offices, common services clause, manifolds |

Every desc runs 101–115 words. The IC return lands a pilot in T03 (`ic_return::PORTS`). The `yard` role
word names the Bunkering Yard here (at Stickney it is the service counter); the yard is where the
Company's ice and reaction mass asks also post. The `sailing plan pad` in T05 rules its throttle line twice
as wide as the rest.

**Listings** (lane v8's `titan_port`; every vendor's agent is `titan_port.clerk`):

| Org (`id`, capital) | Signatory | Storefront (in T15) | Stock | Asks | Bids |
|---|---|---|---|---|---|
| Titan Concession Company (`titan_concession`, 50,000cr; `company`) | Ysolde Marangoni-Teague | `item.concession_manifold` "Concession export manifold" | carbon billet 600 t; water ice 1,800 t; reaction mass 5,000 t | carbon billet 250cr/t (exchange); water ice 40cr/t (exchange, yard); reaction mass 30cr/t (exchange, yard, dockmaster) | — |
| Concession Stores (`concession_stores`, 50,000cr; `company`) | Barnabas Oyelowo-Krauss | `item.stores_locker` "Company Store locker" | provisions 2,600 crew-days | provisions 20cr a crew-day (commissary) | — |
| Selk Townships Board (`selk_townships`, 1,350,000cr; `townships`) | Saoirse Tupou-Lindahl | `item.townships_cage` "Townships receiving cage" | — (bid-only) | — | structural alloy stock 760cr/t, intake 720 t; instrument boards 1,500cr/t, intake 480 t (receiving, exchange) |

The Board carries the port's emergency block (provisions only; ratio 1,500 ‰, intake 100); the premium is
`{300, 500}`; routes `deimos` → `ligeia_voltaire`, `luna` → `ligeia_aitken`. The Townships' cage carries
the Board's manifest and the Company's padlock.

### 7.4 The cast

| entry | Name (as authored) | Room | House / office | Interfaces | Tier | Kind | Knows (the pack's `knows`, condensed) |
|---|---|---|---|---|---|---|---|
| `npc.marangoni_teague` | Ysolde Marangoni-Teague | T07 | Sales Clerk, Titan Concession Company; **signatory** of the billet, ice and reaction mass asks | market, atlas | minor | cast | the Company's asks and what remains, at the Company's price; the ice is overburden sold at its handling cost; the cheapest reaction mass in the system; the Townships' counter down the stair; the layout |
| `npc.oyelowo_krauss` | Barnabas Oyelowo-Krauss | T09 | Storekeeper, Concession Stores; **signatory** of the provisions ask | market | minor | cast | the posted price per crew-day and the crates left; two aboard need twice the days, most arrivals single-handed; the book in one currency, settled through Luna |
| `npc.tupou_lindahl` | Saoirse Tupou-Lindahl | T08 | Purchasing Delegate, Selk Townships Board; **signatory** of the alloy and boards bids | market, help | minor | cast | the Board's bids and intake; everything breaks at ninety-four kelvin; the one counter the Company does not own; the petition to Shackleton lodged at the Registry; the leash; alloy and boards come from Luna |
| `npc.abara_vos` | Kenji Abara-Vos | T05 | Dockmaster of Ligeia Terminal | atlas, travel | minor | cast | the motto and `throttle=450`, one soul, on every listed leg; two aboard only in a good season; the collars and IDLE; the slow board; no relay half; fill the tank before leaving |
| `npc.adichie_brun` | Noor Adichie-Brun | T03 | Lift Marshal, Titan Concession Company (the arrivals desk and the lifter schedule) | atlas, help | extra | cast | the arrivals forms; the lifter windows over Ligeia; the lifters' loads; the Charter notice; the layout |
| `npc.quayle_ibekwe` | Ottoline Quayle-Ibekwe | T10 | Keeper of the Ninety-Four | none | extra | cast | the Charter of 2291 and its lapse in 2390; the opinions on renewal; the custom of the sides; the bar's name; Malapert's decennial quote for a relay half; the current notices |
| `npc.varela_nwosu` | Augustin Varela-Nwosu | T15 | Chief Factor, Titan Concession Company | none | major | principal, `enabled:false` | not voiced in the slice |
| `npc.asante_pryce` | Hallgerd Asante-Pryce | T15 | Speaker of the Selk Townships | none | major | principal, `enabled:false` | not voiced in the slice |

Every cast NPC carries a `persona`, a `speech_style`, `does_not_know` bounds and a non-LLM `fallback`
list, and the tags `corridor_trade` and `port_ligeia`. No doc 11 principal is placed here.

### 7.5 Lore pool (`lore.port_ligeia`, tag `port_ligeia`; the cast may draw `corridor_trade`)

*Public* (nine): The Saturn Charter · Ligeia Terminal · The Company and the townships · No relay half ·
Ninety-four kelvin · The Company's price · The overburden · Come slow or don't come · A company town.

*Guarded* (three): The store's margin · Luna's board price · The leash.

*Secret* (one): `secret.retainer`, `reveal: staff`, about `npc.varela_nwosu`. Stored as a Secret; its
truth is in the pack and in no prompt and not in this brief.

### 7.6 Ambience

Twelve lines: a lock's breath of frost down the concourse, the spar ticking into Titan's shadow and the
heaters coming on together, a Dray's callsign read out with "one soul aboard", a lifter's plume climbing out
of the haze, Luna's prices arriving over an hour old, a bunkering pump's thud, a new name on the slow
board, a bulb of tea caught in the Ninety-Four, the scrubbers briefly clearing the naphtha, the rings
cutting the long window, a Company notice pressed flat by a glove, and frost creeping along a seal.

### 7.7 Geometry, exits and walls

Neighborhood footprint [[100,80],[220,200]], site and structure [[110,90],[210,190]]; budget target 12,
maximum 16. **Company Deck** (floor 1, z 7): T01 [110,100]–[210,115]; T02 [110,115]–[140,140]; T03
[110,140]–[140,165]; T04 [140,115]–[185,165] (circulation); T05 [185,115]–[210,140]; T07
[185,140]–[210,165]; T09 [110,165]–[140,190]; T10 [140,165]–[175,190]; T06 [175,165]–[210,190]. **Cargo
Deck** (floor 0): T11 [140,140]–[175,165]; T08 [175,140]–[210,165]; T15 [140,165]–[175,190] (restricted,
under T10). **Cross-deck:** T07↔T08 public stair, `portal.at` (195,152,7)–(195,152,0), `dz_m` −7 (inside
both rooms); T11→T15 hatch on their shared edge (y 165, x 140–175), `lock: staff`, `visibility: hidden`.
**Walls**, the four exitless public seams: T01|T04 (y 115, x 140–185), T01|T05 (y 115, x 185–210),
T05|T07 (y 140, x 185–210), T10|T06 (x 175, y 165–190), each z 0–4.5. **Counters:** T03
(112,163)–(138,163), T05 (187,117)–(187,138), T08 (178,162)–(208,162) on the Cargo Deck, T09
(112,167)–(138,167). Topology: the gallery's `port.dock` seed → T02 → T03 and T04 (joined to each
other); T04 → T05, T06, T07, T10; T03 → T09 → T10; T07 → T06 and the stair → T08 → T11.

| entry | From → to | Mode | Way in (`names[0]`) | Way back (`names[1]`) |
|---|---|---|---|---|
| `exit.t01_t02` | T01 → T02 | door | `Pressure Lobby <PL>;pl;lobby;in` | `Berth Gallery <BG>;bg;gallery;berths;out` |
| `exit.t02_t03` | T02 → T03 | door | `Arrivals <AR>;ar;arrivals;company` | `Pressure Lobby <PL>;pl;lobby;out` |
| `exit.t02_t04` | T02 → T04 | door | `Concourse <C>;c;concourse;spar;in` | `Pressure Lobby <PL>;pl;lobby;locks` |
| `exit.t03_t04` | T03 → T04 | door | `Concourse <C>;c;concourse;spar;in` | `Arrivals <AR>;ar;arrivals;desk` |
| `exit.t04_t05` | T04 → T05 | door | `Dockmaster <DM>;dm;dockmaster;office` | `Concourse <C>;c;concourse;out` |
| `exit.t04_t07` | T04 → T07 | door | `Sales Floor <X>;x;exchange;sales;market` | `Concourse <C>;c;concourse;out` |
| `exit.t03_t09` | T03 → T09 | door | `Company Store <CM>;cm;commissary;store;stores` | `Arrivals <AR>;ar;arrivals` |
| `exit.t04_t10` | T04 → T10 | door | `Ninety-Four <NF>;nf;ninety;commons;bar` | `Concourse <C>;c;concourse;out` |
| `exit.t04_t06` | T04 → T06 | door | `Lifter Gallery <LG>;lg;lifter;windows` | `Concourse <C>;c;concourse;out` |
| `exit.t07_t06` | T07 → T06 | door | `Lifter Gallery <LG>;lg;lifter;windows` | `Sales Floor <X>;x;exchange;sales` |
| `exit.t09_t10` | T09 → T10 | door | `Ninety-Four <NF>;nf;ninety;commons;bar` | `Company Store <CM>;cm;commissary;store` |
| `exit.t08_t11` | T08 → T11 | door | `Bunkering Yard <BY>;by;bunkering;tanks` | `Receiving <RC>;rc;receiving;townships` |
| `exit.t07_t08` | T07 → T08 | stair, dz −7 | `Receiving Stair <RS>;rs;receiving;townships;down;d` | `Sales Stair <ES>;es;exchange;sales;up;u` |
| `exit.t11_t15` | T11 → T15 | hatch, hidden, `lock: staff` | `Service Spine <SS>;ss;spine;hatch` | `Bunkering Hatch <RH>;rh;bunkering;out` |

Fourteen exits; `exit.t11_t15` is the locked one. The concourse holds two ways back toward the locks,
`locks` to the lobby and `desk` to arrivals, so neither shares an alias with the other.
