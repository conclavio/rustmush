# 20. Surface mining — the quarry loop: a terrain vehicle on a body's ground

**Draft 0.3 — 2026-10-19** (Drafts 0.1 and 0.2 the same day, each folded after a verification pass:
119 findings, then 60). The owner's ask, verbatim: *"once upon a time there was this game called
Starflight ... you had a ship, you had a Terrain Vehicle (TV), you flew the ship between star systems
that had planets, you chose a planet, you got a mercatur projection map of the planets surface, you
landed your TV on it, you drove around and mined minerals, you could scan to find the deposits, drive
over to one and hit mine and you'd get stuff depending on where you were.. then when the TV storage
was full you went back up to the ship to offload and repeat .. that's one of the gameplay loops i want
to support, you fly to an asteroid, moon, planet and then you mine it .. surface mining via a vehicle,
or potentially drone-based mining of asteroid fields, but we'll start with the surface version .. help
me flesh out what this whole mining system could/should look like as a design plan"*.

The end state is already on record, in the owner's ruling R-B of doc 17: *"a lance may go out to mine
(one armed escort and AI miners) ... large IC region maps where units go out, mine, haul, fight and
build, return to base, empty their cargo, sell or refine."* This document is the first phase of that:
the hand-driven surface loop on Ceres and one outer commons, built on what the tree has, with the
drone phase and the fight reserved by shape rather than built. It was written from five independent
designs (player's seat, engine reuse, world and economy, async and scale, knowledge first), fifteen
judgements and a completeness pass over the tree at `4465bf06`, then verified twice against the tree;
the designs, judgements and findings are kept in the session's plan data, not in the repository, and
every file:line below was read this pass.

Decisions are **D-20-n**, inline where they are taken and listed in §14; the owner's open questions
are **Q-20-n** (§15), each with the default the build takes. Cuts are **SM.n** (§13). No deploy without
the owner.

---

## 1. The loop, from the player's seat

At world rate 24 one real hour is one IC day (`crates/tacsim/src/port/readiness.rs:37`), so every
beat below is an IC duration that the rate turns into the right real pace. Every beat is a previewed,
committed act with a due instant; none is a trigger to mash (the owner's rule of 2026-10-02: support
the decision, not the trigger). **One rule for every `+tv/…` act**: `+tv/<act> …` prints the preview
and commits nothing; `+tv/<act> … go` commits exactly what the preview printed. Every act's
completion prints one line to the operator in line mode ("Arrived at C117.", "Cycle 2 of 4: 2 t metal
ore; hopper 4 of 12 t."), the panel keyframe beside it.

1. **Kit up at the Freeport** (once a week, three real minutes). At the Registry and Co-op desk (C05)
   `+licence/buy` takes a prospecting licence (2,500 cr, 168 IC days, seven real days; bought while
   Active it extends from the current expiry, and it is a public contract with the Registry, so it is
   accepted from anywhere); `+coop/refit prospector` swaps the charter onto a **Dray-P**, the refit
   with a two-slot vehicle bay (refused under way or with cargo aboard); at Occator's counter
   `+tv/lease` takes a **terrain vehicle** into the bay (1,400 cr a 168-IC-day term, re-debited at the
   next dock or the 5,000 cr deposit forfeit, the deposit back on `+tv/lease return` with the TV in the
   bay and the hopper empty), and a second lease a second TV. Provisions for the stay go in the locker.
   The desk's `look` lists the verbs, as the Co-op desk lists `+coop/enrol`. Decision: is the licence
   worth it this week, one rig or two, and how long a stay to provision for.
2. **Pick a body, then a site** (one to two real minutes). `+chart` lists sites as destinations with
   their passage now and the Registry's survey word for each ("Rheasilvia: metals fair–rich, strategic
   lean (survey); passage 2 d 4 h; licence 4 d left"): Ceres's by a hop of minutes, Vesta's commons by
   a passage of days whose length the season sets, to richer ground. `+chart/ground map ceres` draws
   the body as an equirectangular grid with that survey as a wash, the port and the sites by letter;
   `+chart/ground ceres` lists the sites with their survey words first, their tenure (commons, open
   blocks, held blocks) and the viewer's own best reading at each. Decision: the worked-over commons
   next door, fresh open ground farther off, your own block, or Vesta when the sky favours it.
3. **Hop and settle** (four to five real minutes). `+ship/plan ahuna`, then `+ship/lift ahuna` (an
   alias of `+ship/undock`, docked or settled) files the hop like any passage; it burns a tonne and a
   crew-day a soul. Meanwhile `+registry ahuna` and the ground chart; the arrival line calls the pilot
   back: "The Dray-P settles onto the pad at Ahuna Flank. Stores 96 crew-days: 3 d 22 h here before
   short rations." Decision, before lifting: sell what is in the hold now, or fill it further first.
4. **On the ground.** `+tv/scan`, `+tv/sound` or `+tv/drive` from the bay imply the launch (300 IC s,
   twelve real seconds, added to the preview); `+tv/launch` is for a pilot who wants to stand on the
   ramp first. The site map opens around the TV: terrain glyphs with footing (regolith, rock, ice) and
   craters, the hull as `H`, the TV as `@`, no ore anywhere yet; `+tv` reads "block C (open), field
   C-NW, footing regolith — no readings here yet: +tv/scan reads the nine fields around you."
5. **Scan** (600 IC s, 25 real seconds). `+tv/scan` reads the nine fields (320 m) around the TV's own
   — 48 cells on each axis — that the mast can see: which family and how rich, as barren, lean, fair
   or rich with the count of ore pockets the word stands for, each field by name (`C-NW`). Footing
   sets sharpness: from rock a single word, from regolith a two-word band ("fair–rich") that never
   excludes the truth, from ice "no return". Decision: sound here, or drive to rock and scan sharper.
6. **Sound** (1,200 IC s, 50 real seconds). `+tv/sound` reads every pocket (40 m) within sixteen cells
   on each axis — a whole field from its middle — as barren, lean, fair or rich, drawn as `.` `1` `2`
   `3` (the digit is the pocket's class: tonnes a cycle until the pocket is drawn), each with a site
   address (`C117`). Everyone on the site hears a sounding as a bearing and a range band; `+tv/sound
   quiet` reads eight cells and is heard by nobody. Decision: which pocket, and whether rivals may
   learn where you think the ground is good.
7. **Drive** (five IC s a cell on regolith: 500 m in ten real seconds). `+tv/drive C117` previews the
   path, the time, the charge it costs and the charge left after the trip home; `+tv/drive C117 go`
   commits a frozen leg (a block, a cell or a bearing and a distance are targets too, for exploring).
   The glyph moves along it; "Arrived at C117."
8. **Mine** (1,800 IC s a cycle, 75 real seconds; rock takes twice as long). `+tv/mine` previews the
   run from your own reading of the pocket — "C117 fair metal ore: 4 cycles, 2/2/2/1 = 7 t, 5 min;
   hopper 0 → 7 of 12 t; charge 71 → 63 % (55 % after the trip home)" — and `+tv/mine go` commits it:
   with no number, this pocket out or the hopper full, whichever comes first; a named count that does
   not fit refuses and names the count that does. On a pocket you have no reading of it prints "No
   reading here: the first cycle will tell." and commits one cycle, whose core is the reading. While
   a run drills the pilot may scan or sound from the new stand, preview the next drive, read the
   registry, or drive the second TV. Decision every pocket: work it out or move on.
9. **Hopper full, go home** (a minute). `+tv/recover` previews the trip to the ramp, the entry into the
   bay and the unload as one command ("home in 25 s; then 12 t metal ore into the hold, 74 t of
   room") and `+tv/recover go` files them; `+tv/unload [qty]` stays for a partial unload. The charge
   recovers in the bay. Decision: another trip, or enough for today.
10. **Stake it, if the ground is good** (one real minute). From the TV standing in an open block,
    `+claim/stake` previews — "block C, your readings: C-NW sounded, C117 cored; soundings heard on
    this block before yours: 2; 2,000 cr now and 2,000 a term" — and `+claim/stake go` files it on the
    strength of your own hand-made readings. The block turns `#` on everyone's site map and only your
    rigs may mine it. Renew it with `+claim/renew` (2,000 cr) once 50 t are worked in the term, or let
    it lapse and it reopens.
11. **Leave it running** (one real minute). `+tv/plan C117,C118,D` files a fill plan: work these
    pockets in order (a block letter sounds its four fields and mines every pocket that reads fair or
    better, nearest first), recover and unload at every full hopper, park when the hold is full or
    the list is worked out, within 288 IC hours; the captain's queue gains `await tv`, so the hull can
    lift, fly home and sell while you sleep. One hold a plan, no more, and a plan is filed by hand.
12. **Sell** (at Ceres). `+market/sell` from the docked hold into the Exchange's ore intake or Occator's
    ice intake — the standing bid takes a whole hold at once and the sell preview prints what it has
    left — and the mill turns ore into the concentrate and water the trade run already sells. Nobody
    buys ore anywhere but Ceres.

---

## 2. What stands, what this document decides

**Stands and is built on.** `mush-industry` (E7 cuts 3–4; 10b §3.3–3.4): an `OreDeposit` whose
live reserve is a `mush-inventory` pool at the deposit's own endpoint
(`crates/mush-industry/src/families.rs:219-264`), the monotone `YieldCurve` and accessibility
(`record.rs:311-358, 557-591`), per-principal `SurveyBands` as knowledge not truth, `ClaimRecord`
with its contest, pledge, suspend and seize states, the extraction licence as an Entitlement account
checked by the pure predicate `extraction_authorized` (`licence.rs:156-232`), the one-bundle cycle
(`families.rs:755-890`), `FacilityRecord` on any object (`facility.rs:51-80`), and **no RNG anywhere**
(`lib.rs:37-46`). `mush-inventory`: pools, lots, `Grade`, `Lineage`, `transfer`, `transfer_marked`,
`mint_origin`, the capacity seam. `mush-atlas` (doc 14): surface regions, the pure background
(`background.rs:20-42`), features, windows, the `atlas.map` frame, the locomotion table
(`crates/mush-atlas/content/locomotion.json`), `JourneyRec` with `Traveler::Carrier`
(`travel.rs:132-153`), hydration's footprint reservation (`hydrate.rs:1-14`). `mush-sim`: the placement
graph, interiors, mounts and compartments, passive domains (uncounted against the eight windows,
`window.rs:19, 199-243`), the carrier journey and the 1 s journey tick (`journey.rs:69-79`: visit 64,
work 16). `tacsim`: the lane pack (bodies with Horizons element tables and moons, places, ports,
routes), `plan_passage` and the flip law, the ticket that freezes a plan, the port tick and the
captain's queue (`port_tick.rs:76-80`, `port/executor.rs`), the goods table and the capacity function
(`port/capacity.rs`), the stock clerk's mint-and-receipt pattern (`port/stock.rs:333-438`), the chart
panel (`port/chart_panel.rs`) as the freshest pulled grid feed, the TUI's lettered screens and the
web's `FeedScreen`. The arena (doc 16) and the minds (doc 17) stand untouched: **the arena hosts
fights, not chores** (D-20-1), and D-16-3's rule that an OOC match crosses no IC boundary is not
reopened here.

**What is not built today, and this document builds** (the gap list the readers verified):
production never wires an extraction licence, so every `@extract` refuses
(`crates/tacsim/src/composition/mod.rs:535-541`; `mush-industry/src/module.rs:138`); the deposit pack
entry's applier is a no-op (`tacsim/src/content.rs:743-775`); opening a deposit is staff-only
(`mush-industry/src/provider.rs:1449-1454`); a cycle has no cadence, no locality and no capacity
check; a deposit's place is an object with a lifetime cap of eight bodies; there is no scan that
discovers; there is no raw-ore good and nobody bids for ore anywhere (`lane/ceres_phobos.json:2174-2239`);
a hull arriving anywhere without a port map is stranded (`port/ship.rs:240-263`); bodies carry no
physical data; a same-body leg is planned at distance zero and burns nothing (`port/passage.rs:493-494,
645-647`); a carried vehicle is not in the payload; `LogisticsDef` demands an interplanetary drive
(`definitions/logistics.rs:78-94`); line mode prints grids at 78 by 20 (`crates/mush-ansi/src/panel.rs:18-22`)
under the owner's 120-column floor; every TUI screen slot is spent (`mush-net-ssh/src/tui/screen.rs:27`).

**The one decision everything else follows from — D-20-1, the clock.** The surface loop runs
**world-side on the IC clock**, every act carrying an IC due instant and applied at it by a dedicated
1 s Coarse pull tick, `tacsim.surface`, in the journey tick's exact shape. Not the arena: the arena
integrates wall time (doc 16 §3.1), so at rate 24 a tracked crossing of a 2.56 km site at the TV's
2 m/s would take some twenty real minutes and burn eight IC hours of life support, its Quiet tempo
lags a seated driver by up to five seconds, its domain is excluded from every world view, its
snapshot resume rewinds the tail against IC minting, and a non-match sortie lifecycle plus an
IC-crossing rule are two L cuts (the arena map's gaps). Not an atlas window: windows are eight
world-wide and clockless. Not a `DurableSchedule` per excursion for player trips: the schedule driver
polls at 250 ms, defers behind legacy persistence and discovers one candidate per poll
(`mush-server/src/world_task.rs:287, 3471-3487`), which is the production source's budget, not a
player's. The port tick (5 s, sixteen agents a fire shared with the clerks and the queues) keeps the
Registry's and the queue's acts. On the IC clock a 500 m drive is ten real seconds and a mining cycle
75; nothing is RAM-only, a restart resumes at the next fire with no rewind, the site stays visible to
the crew, and logging off is safe.

---

## 3. The ground

### 3.1 Sites and bodies — D-20-2

A **site** is a vendor-less `port` entry in the lane pack with a new optional `site` block. The
validator already admits a port with no vendors and skips the goods rules for it
(`crates/tacsim/src/port/def.rs:115-118, 878-884`); the `PortApplier` opens its passive domain and
presents it in the entry's gallery room (`port/apply.rs:591-611`); `plan_passage`, the ticket, the
carrier journey, `Berth::Docked`, `+ship/undock`, the queue's `depart` and the recall arm all reach a
site with no new travel record. The S change is a validator rule letting a `site` entry author zero
orgs and only a pad room, with serde defaults for `rooms`, `treasury` and `orgs` (`PortDef` is
`deny_unknown_fields`, `def.rs:86-110`). Each site has its own lane `location` (role `field`, `body`
the body, `lat_mdeg`/`lon_mdeg` on it, south negative) and routes both ways to its port; a route to a
same-body site is a hop (§3.3), a route to another body an ordinary passage.

```
"ahuna_flank": { "kind": "location", "def": { "label": "Ahuna Flank", "role": "field",
                 "body": "ceres", "lat_mdeg": -10500, "lon_mdeg": 316200 } },
"ahuna_port":  { "kind": "port", "def": {
  "location": "ahuna_flank", "venue": "ahuna", "area": "neighborhood.ahuna_flank",
  "gallery": "neighborhood.ahuna_flank.room.pad", "routes": { "ceres": ["ahuna_ceres"] },
  "site": { "issuer": "freeport_registry", "zone": "core", "geology": "ceres",
            "tenure": { "default": "open" },
            "blocks": { "A": { "tenure": "commons", "worked_permille": 400 } } } } }
```

The site's ground is an atlas region pack in the shape of `region.ceres_freeport.json`: **256 by 256
cells at 10 m, 2.56 km** — exactly D-16-5's arena window, so a fight at a site later promotes the same
ground with no re-mapping (`MAX_TILE_CELLS` 65,536, `mush-atlas/src/cells.rs:16-20`). The pad room
(`port.dock`) sits at the site's centre cell (128, 128), so no pocket is more than 1.3 km from the
ramp along either axis (1.8 km of driving straight across, about 4,100 cu of regolith) and a first
scan reads a full square; a `background` recipe of regolith with ice and rock in the mix, relief at
most 150 cm so lattice steps stay inside the tracked locomotion's 45 % slope and far inside `i16`; two
to four authored **crater** features per site (`FeatureRec` kind `crater`), so the mast's line of
sight is a decision and not a word. Cells are never edited (D-14-9): no depletion ever writes a tile.
The site is partitioned into **16 blocks of 640 m** (64 by 64 cells, lettered A–P), the unit of
tenure (§9), each block into four **fields** of 320 m named by quadrant (`C-NW`), the unit of
knowledge (§5), and each field into 64 **pockets of 40 m** (4 by 4 cells; 256 a block, numbered
1–256), the unit of mining (§6); a pocket's **site address** is its block letter and number, `C117`,
printed first on every readings line and accepted by every `+tv/…` verb. The site applier lays up
**one place Thing per block** (sixteen a site, owned by `tacsim.sites`, never walkable, receipted with
the deposits): a deposit's place is one object with a per-place census of eight
(`mush-industry/src/record.rs:50`; `families.rs:248-261`), and a block's deposits name their block's.
**Tenure is per block**: the `site` block declares a default and its exceptions — `commons` (licence
only), `open` (stakeable) or `held` (an org's, naming it) — so one site may hold a commons corner,
open ground and an org's worked blocks at once. A block row may also carry `prospectivity` (a class
shift applied to its fields before the pocket pass, so a rim can be authored rich) and
`worked_permille` (that share of every pocket pre-drawn into the cursor at apply, receipted, so a
commons can open worked over with its pits showing).

**Phase-1 content — D-20-14 (Q-20-10).** On **Ceres**: **Freeport Commons** (every block commons,
next to the port, `worked_permille 400`), **Kerwan Floor** and **Ahuna Flank** (open blocks, metals and
carbon), **Juling Shadow** (open, ice-rich, farther), and **Occator Rim** (Occator's: two inner blocks
worked by its drones and the rest of the inner blocks held in reserve; its outer blocks open and
authored rich, each carrying the seeded Pallas Syndicate paper, §9.6). On **Vesta**, a lane body since
doc 19 A.1 whose passage from Ceres is days and seasonal: **Rheasilvia Commons**, every block commons
under the Freeport Registry (doc 10 §4 gives Vesta and Pallas "isolated claims, extraction conflicts"
and doc 11 names no government there), with a geology table Ceres lacks — metals fair to rich,
strategic elements lean to fair, carbon barren — so the passage buys richer ground, the strategic
shelf is Vesta's, and beat 2's "choose a planet" is a real choice between a hop and a passage the sky
prices. Every phase-1 block is ruled **core**: doc 11 §5 seeds Ceres station core and the Occator
claims "core / frontier at the margins", and this document holds the margins core until the fight
exists (§17), amending doc 11 §5 (§18); under doc 10 5A no hostile action happens there without a
war, a sanctioned operation, a mutually accepted scene or an explicit crime, so a TV left out is never
exposed. Phobos waits for a Republic issuer and a Stickney ore bid (phase 2).

**The ground chart.** `BodyDef` gains optional `surface_mg`, `radius_km` and a `geology` table name
(skipped on the wire when absent; the lane digest moves once). **`+chart/ground <body>`** lists the
body's sites with the Registry's survey word per family first; **`+chart/ground map <body>`** prints
the grid (`+chart/body` stays the orbital-elements print A.3 shipped and A.7 extended,
`docs/help/ships.md:80`). The feed is `body.ground`, a pulled grid frame on the `sky.chart` model: an
equirectangular grid of 56 by 28 cells (6.43° each) carrying the **public R0 wash** (§3.2) per family
under a layer key, the port `F` and the sites by letter; beside the grid the vector block lists each
site's survey words, its tenure (how many blocks open, held, commons), its passage now for this hull
and load, and the viewer's own best reading. A cell of a body Ceres's size is some 53 km at the
equator, so a site is a point on it and the landing beat is the lettered sites this phase (the help
says so); the chart decides by what it says of each site. Landing anywhere on the lattice
(`Representation::Implicit` is declared and never read, `mush-atlas/src/record.rs:403-411`) is phase 2.

### 3.2 Geology — D-20-4

The ground's ore is a **pure, keyed, integer function** of a body-fixed position, never a stored
field and never a roll:

```
geology(key, body, level, lattice) -> HMAC-SHA256 over "rustmush.geology.v1" | body | level | x | y
```

HMAC-SHA256 is the primitive `mush-core` already uses for keyed draws
(`crates/mush-core/src/resolution.rs:12-23`); here it has no counter, no stream and no transcript, so
the same inputs give the same output for ever, the RNG tripwire stays green and D-E7-6 holds. The
key is a **fourth durable key family** beside the offer, resolution and decision keys (the
`KeyFamily` trait, `crates/mush-config/src/durable_keys.rs:23-35`; the families in `offer_keys.rs`,
`resolution_keys.rs` and `decision_keys.rs`; loaded at `mush-server/src/boot.rs:413-434`): one
version, never rotated, on disk in its own key directory and never in a pack, a frame or the source.
A **fingerprint** of it is pinned in a world record at first use; a mismatch at boot disables the
mining layer out loud on the readiness line rather than silently moving every reserve under the
stored ones (Q-20-12). Only staff read truth, through `@geology <site> <addr>`.

Generation is **top-down, so readings nest**, and one word set — **barren, lean, fair, rich** —
names richness on every surface (the code's P0–P3 and A–D never reach a player): a *province* (one
cell of the body's 56-by-28 chart, 6.43°, 1,568 a body; the R0 level) carries per-family
prospectivity from the body's `geology` table (Ceres: volatiles fair to rich nearly everywhere,
metals lean to fair in provinces, carbon feedstock lean, strategic elements barren with rare lean;
Vesta: metals fair to rich, strategic lean to fair, volatiles lean, carbon barren); a province
conditions its *fields* (320 m, the R1 level: per family the count of ore pockets the field holds —
barren none, lean 1–4, fair 5–8, rich 9–16 — shifted by the block's authored `prospectivity` if any); a
field conditions its *pockets* (40 m, the R2 level: barren, lean, fair or rich, with tonnes lean 3–6,
fair 6–12, rich 12–24). So a block of four fields holds **at most 64 ore pockets per family** by
construction, and at most 64 × 24 = 1,536 t; a whole site some thousands of tonnes per family, most of
them lean. Doc 10 4A's four raw families — metals, volatiles, carbon feedstock, strategic elements —
are all mined (the goods rows in §7.3); "what you get depends on where you are" is the body, the
family, the field and the pocket, not a digit.

The site's terrain composite (background plus features) is built **off-loop** with the atlas's pure
`capture()` and `composite()` (`mush-atlas/src/window.rs:201, 521-688`) from the region pack and
held in a RAM LRU of sixteen (about 384 KiB a site); the site's geology table (4,096 pockets by four
families, two bytes each) is built beside it once per key version. Both are derived, never stored,
re-derived at boot. One finding on the way: the composite casts background elevation with
`i16::try_from(..).unwrap_or(i16::MAX)` (`window.rs:546`; `:470` is the harmless tile-mean twin),
which turns a low into a cliff, and background elevation is `datum_cm + noise` with `datum_cm` an
unbounded `i32` (`background.rs:36`, `record.rs:306`); the recipe validator requires `datum_cm ±
relief_cm` inside `i16` and the cast saturates symmetrically (SM.1).

### 3.3 Landing — D-20-3

With the flip law the Dray's thrust at full throttle is fixed at 206 kN (`accel_mg` 50 at the rated
421 t, `crates/tacsim/content/units/dray.json:83-92`), so **landing is thrust-limited, never
fuel-limited** (escape from Ceres is 0.51 km/s against a 6,000 km/s exhaust). The law: a leg to or
from a site is refused at planning when the hull's thrust-to-weight **at departure mass and at the
plan's throttle** (`port/passage.rs:76-80`) against the body's `surface_mg` is under **1.2**. Ceres
(28 mg) passes at 1.79 laden at full throttle (and from about 670 ‰); Vesta and Pallas pass; Phobos
(0.6 mg) trivially; **Luna (0.30), Titan (0.36) and Mars (0.13) are refused** — a lore-as-constraint
lever that keeps the Dray off the large bodies by data, where a *Hod* lander is a later design. A
hop's distance is an authored same-body `base_s` of **3,600 IC s** through the laden step, as
same-body legs are today (`port/passage.rs:235-244`; `mush-sim/src/route.rs:837-846`): about 5,300 IC s
out with a full tank, one TV and a week's stores (three and a half real minutes), about 7,000 IC s
home with a full hold (five), a suborbital arc across a body 940 km wide. Its burn is the rocket
equation over an ascent Δv plus a gravity loss — today a same-body leg is planned at distance zero
and burns nothing (`passage.rs:493-494, 645-647`) — **ceiled to a whole tonne so a hop is never free**
(`passage.rs:699`'s rounding); crew-days are the existing rule, `occupants × Σ ceil(secs / 86,400)`
(`passage.rs:707-722`), one per soul for a sub-day hop. **Landed life support** (Q-20-7): a hull at
rest on a site debits one crew-day per soul per IC day, so a solo stay of a real day costs 24
crew-days of the locker's 120 and provisioning for the stay is beat 1's decision; the debit **stops
at the stores one leg home needs** for the souls aboard (a once-per-stay "short rations" notice in the
`tacsim:notice` ring), so the hop home is always plannable and a hull landed for ten days still lifts;
the arrival line, `+tv` and `+ship/plan` print the stores and the days they last here. A hop counts
as a departure for the seven-day recall (`port/recall.rs:27-29`), so a week of mining never pages a
dormancy recall. The berth line reads "settled on the pad at Ahuna Flank"; `Berth::Docked` is reused
as is, and `+ship/lift <port>` is `+ship/undock`'s alias docked or settled.

### 3.4 Depletion — D-20-5

The reserve is **one `OreDeposit` per (block, family)**, its commodity **the family's ore good**
(§7.3), its live reserve the inventory pool at the deposit's endpoint, **credited once when the site
is applied** — never on the player's path — by a composed **system opener**:
`IndustryModule::with_system_opener(..)` admits `DepositOpen` from the applier's own actor (an object;
staff at apply, never a player) with the site's server-authored rows as fixed parameters and a
receipt per `(site, block, family)` as the replay guard, so a re-apply credits nothing (the stock
clerk's mint-and-receipt shape, `port/stock.rs:333-438`); `tacsim.sites` is the **owner** principal
of the block places, the deposits' place objects and the spoil vaults, not a submitting actor (an
authoritative submission's actor is an `ObjId`, `mush-core/src/module.rs:4964-4975`). Each block
deposit is opened with accessibility 100 and a one-point curve `[{cumulative 0, per_cycle 1}]`, so
`extraction_authorized`'s body test (`licence.rs:167-172`) passes until the pool is empty and the
pocket cursor alone sets the tonnes. The reserve is the sum of the block's pocket tonnes for that
family (less any authored `worked_permille`), computed off-loop from the geology table and
re-derived server-side, never player input. Up to four deposits per block place and 64 per site sit
far inside `MAX_ORE_DEPOSITS_PER_PLACE` 8 and the 4,096-deposit walk.

A **pocket cursor sidecar** per deposit, `tacsim.pockets:<deposit>`, holds at most 64 rows
`{pocket, drawn_t, lock}` — bounded by the 64-pockets-per-block construction, so the cap never binds
on ordinary play — written in the same bundle as every cycle; **the lock's one home is the sidecar
row**, keyed by pocket, so a second rig's start reads it in one `get`. **The invariant**
`initial_reserve − pool == Σ drawn` is checked by industry's 60 s reconcile sweep
(`mush-industry/src/store.rs:634-682`) through a trait declared in industry and composed at the root,
as `RecipeAuthority` is (`composition/mod.rs:538-540`): `ExternalDraws::drawn(world, deposit) ->
Option<u64>`, asked for every deposit whose `external` is set; a mismatch quarantines the block
fail-closed. `OreDeposit` gains that optional `external` provider name, so industry's own
`@extract/start` refuses a block the surface provider works; nothing drains a block pool behind the
sidecar. A pocket's class is fixed at generation and its digit never changes; what changes is what is
left, which the mine preview says ("2, then 2/2/1") and the public **pits** show — `v` worked, `V`
exhausted — an overlay derived from the cursor, never a cell edit and never a feature. A block whose
every pocket is exhausted reads "worked out" on the chart for everyone; the reserve's figure stays
staff-only.

---

## 4. The vehicle

### 4.1 What the TV is — D-20-6

`crates/tacsim/content/units/tv_prospector.json`, a full `UnitSpec` (the schema is
`deny_unknown_fields` and demands sections, facets, silhouettes, components, mobility and
`sensor_height_cm`, `definitions/unit.rs:832-849`), so that a later cut can field it: sections
`chassis`, `hopper`, `boom`; components `power` and a `visual_sensor`; mobility `tracked {
max_speed_cms 300, accel_cms2 50, brake_cms2 100, turn_rate_mdeg_s 20000, reverse_factor_pct 50 }`,
data only (`TrackedDef`, `unit.rs:580-594`). On the locomotion table's tracked row (`regolith 150`,
`rock 300`, `ice 200`, `crates/mush-atlas/content/locomotion.json`) that is 2.0 m/s on regolith (five
IC s a cell), 1.0 on rock and 1.5 on ice. Its class is a **new `UnitClass::Vehicle`**
(`unit.rs:726-775`), unfieldable in phase 1 exactly as `Hauler` is — the lobby never offers it and the
custody seam refuses it — and flipped fieldable by the promotion cut (§17), which the `Hauler` class
could never be.

**Logistics** is a stowage-only variant: `LogisticsDef.drive` becomes `Option` (`logistics.rs:78-94`)
and a drive-less unit never plans a passage; `dry_mass_t 6`, `berths 1` (the cab), `stowage { hopper:
{ accepts: [stow.ore], limits: { si.mass_kg 12000, si.volume_l 13200 } } }`, `payload_ceiling_kg
12100`. The **interior** is the two rooms the schema's floor demands (`MIN_INTERIOR_ROOMS` 2,
`logistics.rs:58`): a cab and the hopper store. In phase 1 the cab is empty — **the TV is
teleoperated** from the settled hull's bridge under the operator's control lease (Q-20-4 keeps the
crewed cab open; it is the one place a Starflight veteran will feel the difference, and the engine's
answer — a body in the cab, seat reach, a rescue rule — is a later cut, not a different TV).

**Tools** are a new content kind, `crates/tacsim/content/tools/*.json`, read by the surface engine
now and lowered to `ComponentRole::Tool` at promotion: `drill_core_mk1 { cycle_s 1800, rock_factor
2, energy_cu 2000, yield_t: { lean 1, fair 2, rich 3 } }` and `scanner_mk1 { scan_s 600,
scan_radius_cells 48, mast_cm 250, scan_cu 1000, sound_s 1200, sound_radius_cells 16, quiet_radius_cells
8, sound_cu 2000 }`; every radius is **per axis** (a square: the scan's 48 reaches the nine fields
around the TV's own from its middle, the sounding's 16 a whole field, the quiet sounding's 8 a
quarter). **Charge** bounds a run, not a shift: 100,000 cu (one cu is four watt-hours), spent by the
drill, the scanner and driving (15 cu per cell of nominal cost: a kilometre of regolith is 2,250 cu),
restored at 50,000 cu per IC hour **in the bay**, the unload stop included (a 600 IC s unload regains
8,300 cu), so a full hold's worth of cycles never drains it and the battery binds long drives and long
plans only. **The reserve rule:** every drive, scan, sounding and mining preview shows the charge
after the act *and after the trip home*, and the act is refused when that would fall under 5 %; a TV
cannot be stranded by its battery, by construction.

### 4.2 The hopper vault

A single-room store interior anchored on the TV and a vault on it, laid up by the hull recipe
(`declare_space`, `port/hull.rs:685-714`; `create_vault` as `+coop/enrol` does,
`port/coop.rs:1520-1553`), **owned by the system principal `tacsim.vehicles`** — never the pilot and
never the hull's titled owner (a graduate's hull is titled to the player, `hull.rs:797-801`). No
player passes `may_take` on it, so `live_endpoints` never offers it, `@inv/move` cannot teleport ore
between hopper and hold, and doc 18's one-vault premise (D-18-37) survives without building P-H.
Only two server-authenticated acts write it: the cycle's transfer in (§6) and the unload out (§7).
Every quantity in it is a pool, so gauges, capacity and payload read it at once.

### 4.3 Riding the ship — D-20-7

`crates/tacsim/content/units/dray_p.json`, the **Dray-P**: the Dray with a `bays` entry on its
`InteriorDef` — a single-room space anchored `MovingObject { host: hull }` with a compartment
`maximum_items 2` and an `EgressSpec` at the ramp pose — and cargo **86,000 kg / 103,200 L accepting
`stow.bulk` and `stow.ore`**; tank, locker and berths unchanged. Cargo drops from 100 t to 86 t: room
for two TVs (12 t) and 2.3 t of headroom under the 181 t rail at the stowage maxima with two
occupants (178,680 kg), since every hopper is empty at every departure by `HOPPER_LOADED`;
`payload_ceiling_kg` and `accel_mg` are untouched and every pinned passage figure stands. `payload_kg`
(`port/passage.rs:733-757`) learns to add a bay's units (their dry mass) and their hopper pools; **a
Dray-P is never re-rated**. A TV is placed `Interior { bay, slot }` and rides the carrier subtree for
free (`mush-sim/src/journey.rs:3-9`). `+coop/refit prospector|standard` at the desk swaps an enrolled
charter between the Dray and the Dray-P as lane data (refused under way or with cargo aboard). A TV
comes from **Occator's lease** — doc 11 §7's units table leases Occator's extractor drones to
claim-holders, and a terrain-vehicle lease extends it (Q-20-6): `+tv/lease` at Occator's counter is the
player's own act under the server-authored lessor, the `+coop/enrol` shape (`port/coop.rs:1-27`),
1,400 cr a 168-IC-day term (re-debited at the next dock, else the deposit is forfeit) and a 5,000 cr
deposit held, refused without an Active licence or a free bay slot, laying the TV into the bay;
`+tv/lease return` refunds the deposit with the TV in the bay and the hopper empty, and is refused
elsewhere. Two leases are two TVs; the second is the lever that makes a live hour a juggling act.

`+tv/launch` is an egress to `Direct` on the site's passive domain at the ramp pose
(`mush-sim/src/commands.rs:2060-2159`), legal only while the hull is settled at a site and the
operator is aboard, and implied by the first `scan`, `sound` or `drive` from the bay; `+tv/recover`
(§7.1) places it back. Two typed **departure refusals** keep the frozen ticket honest: `TV_OUT` (a bay
slot's TV is on the ground) and `HOPPER_LOADED` (a hopper holds ore: unload or dump first). `+tv/dump`
moves the hopper's pools to the site's **spoil** pool, a system vault per site — conserved,
unrecoverable, a decision with a preview. An **excursion** runs from launch to park and is identified
by `ExcursionId (tv, launch_ic)`: the sortie identity doc 17 needs (D-UM-45, a hangar to a hangar), the
bay being the hangar.

---

## 5. Knowing — D-20-8

A player never sees the ground; they see **their own readings**. The store holds *coverage and
provenance*, not readings: one chart document per `(principal, body)`, `tacsim:chart:<principal>:<body>`,
with at most 1,024 field rows `{field, fidelity, first_local_at, last_at, provenance ≤ 4}` and at most
64 pocket blocks of 16 by 16 one-byte cells (tier and pit seen), some 70 KiB worst case; at the caps
the server refuses and `+tv/forget <site>|<body>` frees room. Readings are **re-derived at render**
from the cached geology under the row's pinned reading-rule version, so a reading never drifts from
the ground and the store never holds truth. The `+tv` status line always says where the TV stands —
block, field, footing — so a scan's rows and a stake's block are never a guess.

Three tiers, each a pure quantisation of the geology, in the one word set:

- **R0, the Registry's orbital survey** — public, the same for everyone, the ground chart's wash and
  the survey word on every site line.
- **R1, the scan** (`+tv/scan`, alias `+tv/survey`; 600 IC s, 1,000 cu) — the nine fields around the
  TV's own, 48 cells on each axis, that the 2.5 m mast can see over the composite (the atlas
  `line_of_sight` walk, `mush-atlas/src/trace.rs:606`, which reads terrain and crater features; the
  `CellGrid` walk at `:67-77` reads cells only), at field fidelity: family and richness, with the
  pocket count the word stands for, by field name. **Footing** sets sharpness, never range: from rock
  a single word; from regolith a two-word band ("fair–rich") that never excludes the truth; from ice
  "no return". The preview names the footing here and what better footing would buy ("from regolith:
  fields read as a band; rock 60 m W"). Where you park is the prospecting decision.
- **R2, the sounding** (`+tv/sound`, 1,200 IC s, 2,000 cu) — every pocket within sixteen cells on
  each axis, a whole field from its middle: barren, lean, fair, rich, and its pit state. A sounding is
  **heard site-wide** as a bearing and a range band by every party on the site (the Epistemic
  Engine's rule that active perception is observable; Q-20-9), at most one line per sounder per IC
  hour and otherwise a transient mark on the site map's row of other rigs (`+tv/quiet` drops the
  lines). `+tv/sound quiet` reads eight cells and is heard by nobody, at the same cost — the decision
  beat 6 names. A scan is passive. Mining a pocket also writes its R2: the core is a reading.

Repeating an instrument from the same cell is refused at no cost: "You learn nothing new here." —
industry's "a re-survey cannot narrow" (`mush-industry/src/commands.rs:379-433`) turned into play; the
only ways to narrow are a finer tier, better footing or a closer stand. **Provenance** is the chart's
own enum, modelled on `mush-propagation`'s but never changing it (`record.rs:663-677`, a side
projection that gates nothing): `ChartProvenance::Local { instrument, at, via: Hand | Plan }` or
`Reported { source, at, via: Gift | Sale | Salvage }`; only a `Local` row made by hand, at R1 or
finer — a scan, a sounding or a core — counts as discovery evidence for a stake, and only a hand-made
sounding the site heard for a contest (§9.3–9.4); a row a plan's instruments wrote while the operator
was away is the character's own reading for every other use and never priority (§8). Selling or
giving a chart is phase 2 (a market `Representation::Entitlement` listing delivered by a
`DeliveryProvider`, `mush-market/src/record.rs:132-133`, `module.rs:86-105`, with the sale as
provenance). A TV's log ring is its black box; salvaging it yields `Reported { via: Salvage }`: stolen
knowledge, never stolen discovery.

**Fog, by construction.** Every ore glyph on every surface is the viewer's own row. "You have no
reading there." is byte-identical for unknown, uncovered and barren-but-unsounded (the posture of
`commands.rs:52-56`). Nothing lists "nearest ore". Public facts are said plainly — "That block is
under claim.", "Your licence lapsed at 05-21 14:00.", "The hopper holds 2 t more." — and hidden truth is
never said. Industry's `SurveyBands` (lifetime cap of eight surveyors per deposit, `record.rs:52`) are
not used for cell knowledge; a block's `grade_class` and `reserve_class` are recorded at materialisation
for the Registry's folio.

---

## 6. Mining — D-20-9

**The act.** `+tv/mine [cycles]` (the owner's word; `+tv/drill` an alias; Q-20-8) previews first —
**from the operator's own R2 row of the pocket, or a public pit, never the ground**: the pocket's
class, each cycle's tonnes from the remaining tonnage, the run's IC time (1,800 IC s a cycle on
regolith or ice, 3,600 on rock: terrain sets the pace, the pocket sets the tonnes), the hopper before
and after the run, the charge after the run and after the trip home — and `+tv/mine go` commits
exactly that run. On a pocket the operator has no reading of, the preview says "No reading here: the
first cycle will tell." and the run is one cycle, whose completion writes the R2 and prints the class:
the core is a reading, paid for like any instrument, so the preview is never a free sounding. **The
commitment is bounded by construction, never by a repeat and never by a clamp**: with no number it is
this pocket out or the hopper full, whichever comes first (at most eight cycles, by the 24 t cap); a
named count that does not fit **refuses and names the count that does** ("4 cycles do not fit: the
hopper is full after the third — `+tv/mine 3`, or `+tv/mine` with no number"). **No `until=full`, no
repeat across pockets, no auto-fire** for a seated pilot; `+tv/mine stop` lets the running cycle
complete and yield and drops the cycles after it at no cost. Repetition across pockets belongs to the
plan (§8) and, in the drone phase, to sealed minds.

**The start** is an authoritative submission as the operator (`tacsim_surface_mine`), its
`project_offer` holding the world and passing server-authored fixed parameters: the TV stands in the
pocket and is idle; the operator's licence account is Active; **tenure allows it** — the block is
commons, or `extraction_authorized` (`mush-industry/src/licence.rs:156-232`, called pure over the
block deposit's own claim index) answers Ok for a claim **whose issuer is the site's issuer**; the
pocket's sidecar row shows the lock free or held by this excursion; the hopper **admits** the
committed tonnes by `capacity::admits` over the hopper compartment (`port/capacity.rs:214`) —
refused at start, never clamped at completion; the charge covers the run and the trip home. It writes
the lock on the sidecar row and the cycle `{pocket, k, cycles, yield, due_ic}` on the excursion's
state family, once each; the tick debits the charge when it verifies the start.

**The completion** is the surface tick's one bundle at `due_ic`, as the operator:
`inventory.transfer(block deposit pool → hopper pool, the family's ore good, yield)` on tacsim's
delegated inventory families (`composition/mod.rs:510-515`; `mush-inventory/src/pool.rs:468-506`), the
sidecar's `drawn += yield`, the next cycle or the lock's release, and a **receipt** keyed `(deposit,
pocket, k)` as the replay guard; the tick then writes the excursion log, the chart's R2 row and the
operator's line. **Nothing is minted per cycle** and no lot is created: the reserve was credited once
at apply, so conservation *is* the transfer, the hopper never grows toward the 512-lot cap, and there
is no second dissolve bundle. A bounded per-site ring publishes the cycle as the fact
`tacsim.surface.mined` for contracts and commissions (not `industry.extraction.cycled`,
`mush-industry/src/record.rs:88-89`, which is written on industry's own families, which no owner
delegates; only inventory delegates, `tacsim/src/composition/mod.rs:229`). An `IndustryHandleCell`
exported by `mush-industry` (the `InventoryHandleCell` shape) gives the composed provider read access
to deposits and claim indexes; it writes none of them.

**Yield** is the pocket's class — lean 1 t, fair 2 t, rich 3 t a cycle — until the pocket's tonnage
is drawn, the last cycle yielding the remainder; it never rises, so D-E7-6 holds. Grade is spent here
as tonnes (D-20-10): pools stay gradeless, no bid is graded, and the dissolve-then-crystallise
laundering hole never opens. **Two rigs on one pocket**: the lock refuses the second with "Another rig
is working that pocket." — a public fact, since units on the site are drawn for everyone.

**During a run** the TV is held to the pocket, and the pilot is not: a scan or a sounding from the
new stand, the next drive's preview, the registry, the chart and the second TV are all legal. A
45-minute session is some thirty cycles and a decision at every pocket, with the next one planned
while the drill runs.

---

## 7. Offload and sell — D-20-10

### 7.1 TV to ship

`+tv/recover` is one command that files two tick acts with one preview: a frozen leg to the ramp
cell and, at its due instant, the placement `Interior { bay, slot }` (`mush-sim/src/window.rs:325`);
then the unload. It cannot be one bundle — placement is a legacy slot `prepare_authoritative` cannot
reach (`mush-sim/src/journey.rs:11-20`, RESOLUTION A) — so the unload is its own act, `transfer_marked`
from the hopper pools to the hold pool (`mush-inventory/src/pool.rs:508-568`), its quantity previewed
as `min(hopper, max_more(hold plan))` (`capacity.rs:262`) and re-checked at prepare, the marker
derived from `(tv, excursion, ordinal)`; `+tv/unload [qty]` stays for a partial unload and a TV
already in the bay. The intermediate state — TV in the bay, ore in the hopper — is durable and the
departure refusal `HOPPER_LOADED` reads it. **The capacity plan is generalised**, not the sale
predicate: `plan_of(&LogisticsDef)` (`port/market.rs:392-400`) and `capacity::admits`/`max_more` are
read over any unit's stowage block, the hopper's included, so a hopper and a Dray-P hold refuse an
overfill identically, while `governed_hold` (`market.rs:82-97`) stays the programme-hull predicate
the sale rule and `CoopProceeds` read.

### 7.2 Ship to port

Only the **governed hold** sells (`HOLD_ONLY_SOURCE`, `mush-market/src/trade.rs:1836-1844`): the
hopper is refused twice, by that rule and by `may_take` (its owner is the `tacsim.vehicles` system
principal). The Starflight shape — offload to the ship, then sell — is the law. `+market/sell
<listing>=<qty>` at Ceres, or a queued `sell` in the captain's queue, into the ore intakes below; the
sell preview and `+market` print the bid's unfilled quantity, and `sell … all` sells what the bid
takes and leaves the rest in the hold. The Co-op's share applies with one amendment (§7.4).

### 7.3 The ore goods — Q-20-1's default

Mined matter is **raw ore, a good of its own per family**, bought only at Ceres and milled into the
concentrate and water the trade run sells. The alternative — minting the existing `metals` and
`volatiles` rows — makes mine-then-haul-to-Kasei net about 17,000 cr a leg (21,600 less the share,
the fuel and the stores) against the trader's 5,390 on the same passage (doc 18 §4.1), so every
charter stops buying at the Exchange; that is the reason this fork is the owner's. Goods table
**version 2** adds four rows under a new stowage tag `stow.ore` (the locker rule passes because four
goods share the tag, `definitions/goods.rs:384-386`); each is the commodity of its family's block
deposits:

| Good | Family | Per tonne | Hopper (12 t / 13.2 m³) | Ceres bid | Mill | Into |
|---|---|---|---|---|---|---|
| `metal_ore` | metals | 1,000 kg; 500 L | 12 t (6.0 m³) | **Exchange 70/t** | 5 : 2 | `metals` at 175/t cost against the 200 ask |
| `ice_ore` | volatiles | 1,000 kg; 1,000 L | 12 t (12.0 m³) | **Occator 35/t** | 5 : 2 | `volatiles` at 87.5/t against the 100 ask |
| `carbon_ore` | carbon_feedstock | 1,000 kg; 1,200 L | 11 t (volume binds) | **Exchange 50/t** | none in phase 1 | a stock for a later feedstock ask |
| `strategic_ore` | strategic_elements | 1,000 kg; 600 L | 12 t | **Exchange 600/t** | none | the Exchange's prize shelf |

The Exchange may bid `metal_ore` because it asks `metals`, a different good (`port/def.rs:642-644`
forbids asking and bidding one good); Occator bids `ice_ore` beside its `volatiles` ask on the same
rule. Kasei bids no ore: the refinery's feed is concentrate, and miners and traders stay two
professions joined at the Exchange's pool. No reach premium applies, because no port asks ore
(`port/price.rs:130-177`). Occator's capital rises from 50,000 to 300,000 by tranche (lane data); the
Exchange's 4,000,000 covers its intakes.

**The intake drip.** The built clerk re-posts a bid only when its listing is gone *and* its recorded
deadline has passed (`port/stock.rs:784-786`; `posts_afresh`, `port/vendor.rs:157-169`, also demands
the authored intake), and the deadline is the 90-day TTL (`mush-market/src/api.rs:44`), so an intake
filled on day three would close the market for a quarter. A bid row gains `refresh { per_ic_day, cap
}`, and the drip is **its own stock-arm step with its own predicate** — an IC-day boundary crossed
since the row's last post — that withdraws the standing bid and posts through `wanted_spec` at
`min(cap, unfilled + drip)` within the house's capital, rewriting only the row's listing and
`expires` (the emergency arm's shape, `stock.rs:960-975`; `posts_afresh` is not consulted). Drips 40 /
40 / 20 / 2 t a good an IC day with standing caps 240 / 240 / 120 / 12 t (Q-20-11; lane data), so a
standing bid takes a whole hold in one act and six IC days of drip at most accumulate. At rate 24
that is **960 t a real day of each bulk ore**, 480 t of carbon ore and 48 t of strategic ore, and at
the bids a full drip costs the Exchange some 120,000 cr a real day of its 4,000,000 and Occator 33,600
of its 300,000: the capital clause binds Occator in about nine real days and the Exchange in about a
month unless the mill's resale refunds it, which is the point. The drip is the whole ore economy's
size, an owner knob.

**The mill** is P5's supply half (doc 18 §5's post-loop list): a receipted port arm turns the house's
storefront ore pool into concentrate at the ratio and raises the ask's quantity by the proven
surplus through a new market `restock_ask` op; the market still never mints
(`mush-market/tests/e7r2.rs:305-336`). The Exchange's finite 4,000 t of concentrate gains a
replenishing source and the loop closes: miner, Exchange, trader, Kasei.

### 7.4 The Co-op's share — Q-20-5's default

The Trust's 250 ‰ is of the margin over a **reference** read at contract build
(`port/proceeds.rs:15-21`), and D-18-24 fixes a reference as a listing, never a price. Ore has no
origin ask, and the goods-table fallback (`reference_of`, `proceeds.rs:135-180`) would make the
anomaly path the normal path. **This amends D-18-24 for a good no port asks**: a programme `reference`
entry may be a fixed price in minor units (`reference_minor`; port entries are integer-only,
`def.rs:86`) only for a good with no ask anywhere on the lane, which the validator checks; every other
entry stays a listing (§18). The fixed references sit at **half the bid** — 3500 / 1750 / 2500 /
30000 minor — so the Trust takes a quarter of the upper half: 8.75 cr a tonne of metal ore, 12.5 % of
gross, beside the trader's 11 %.

---

## 8. The clock — D-20-11

**The surface tick** `tacsim.surface`: Coarse, 1 s, the journey tick's shape
(`mush-sim/src/journey.rs:69-79`): each fire visits at most 64 excursions whose `due_ic ≤ now`,
oldest due first, works at most 16, and acts **as each excursion's operator through
`Fire::as_principal`** (`port_tick.rs:1043-1059`), whose `arbitrate` admits one act per principal
per fire, so an operator's two TVs complete on two successive fires — the TV's due instant is the
authority, the fire only the check — and licence and tenure are re-checked at every act, a lapse
stopping a plan at its next step with a public line. Acts: launch done, leg arrival (the pose is
written once at arrival; positions between are pure interpolation from the leg record), scan done,
sounding done, cycle complete, recover done, unload done, plan step. A drive's A* runs **once per
command** over the composite with a budget of 65,536 cells, a whole site, passed to the atlas's
`path` (`mush-atlas/src/physics.rs:614-622`; the atlas default `path_max_cells` is 4,096,
`physics.rs:92`; the arena already raises its own to 65,536), never per tick, and the path is frozen
to at most 32 waypoints with cumulative IC seconds. The engine's submit queue (64 actors,
`mush-core/src/engine.rs:732`) is shared, so the tick stops visiting when it holds sixteen acts. Caps:
32 excursions a site, 1,024 world-wide, **four per operator** — the Compact's span-of-control lever;
a Dray-P carries two, so a live hour may be one TV and a 75 s rhythm, or two staggered TVs and a
decision every forty seconds.

**Two records on two lanes.** `tacsim:tv:<obj>` is a **normal-lane document** in the `JourneyRec`
shape — operator, `ExcursionId`, state (`Bay | Out | Held | Parked`), pose, the frozen leg, the
charge, the plan, a log ring of sixteen lines, at most 4 KiB — written in the same Outbox tick batch
as the placement for launch done, leg arrival and recover done, the act ordinal its idempotency key;
`tacsim.excursion` is a **state family** holding the current cycle `{pocket, k, cycles, yield,
due_ic}` and the locked pocket's address (the lock itself lives on the sidecar row, §3.4), written
only inside the mine-start and completion `ActionTxn`s, which the tick verifies by pull the way the
executor verifies an order (a refused completion releases the lock and writes the public line).
Nothing is RAM-only: a restart resumes at the next fire, overdue acts applying at their own stamps, so
the post-image after a restart equals an uninterrupted run (an acceptance test).

**Leaving it running.** `+tv/plan <addr>[,<addr>|<block>…]` files a **fill plan**: work these pockets
in order — a block letter sounds its four fields and mines every pocket that reads fair or better,
nearest first — recover and unload at every full hopper, park when the hold is full or the list is
worked out; the compiled closed vocabulary (scan, sound, drive, mine, recover, unload,
return-on-idle) is what the preview prints with its IC hours and `+tv/plan show` reads back. It names
only pockets and blocks in the author's own knowledge and claims, is **bounded by one landed hold and
288 IC hours** (twelve real hours), one plan per TV, and **it never re-arms**: a human files each
shift, so a player who logs in once a day fills about one hold a day. The captain's queue gains one
closed kind, `await tv [until=<≤288 IC h>]`, completing when the TV parks or the plan ends, so `await
tv; depart ceres; sell metal_ore all` runs the hop and the sale after the hold fills; a `depart` is
refused while a pad's TV is out. With no order for 24 IC hours a TV out returns and parks — a Compact
floor like the recall arm, not a machine decision, and never the queue's bound. Reconnect notices: at
most one a trip plus a summary, in the existing `tacsim:notice` ring (`port_tick.rs:91-99`). This
**extends Q-18-22** (doc 18 §8: "no repeating plans") rather than breaking it — a fill is one fill of
one hold, as a queued buy's `fill` (D-18-41) completes one order and never loops — and records the
reading of **T13** this rests on: offline ore, and the readings a plan's instruments and cores log,
are *owned-asset production and the character's own instruments on a sortie the character dispatched*
(doc 17 R-B's lance), through a plan the character signed, exactly as the queue's offline sells are
the pilot's own signed orders (D-18-4), and never a seat; a plan-written reading carries `via: Plan`
and is never discovery evidence (§5). The owner rules on the bound (Q-20-3).

**The seal.** A fitted mind is sealed at launch and never by itself (doc 17 §12, D-UM-45); in phase
1 no mind drives a TV, and the excursion carries the sortie identity so the drone phase plugs in
without re-keying.

---

## 9. Tenure and law — D-20-12

### 9.1 The licence

`tacsim.licence.freeport`, an `InstrumentKind::Entitlement` instrument, finally wired at the
composition root by `with_extraction_licence` (`mush-industry/src/module.rs:138`), which production
never calls today. `+licence/buy` is **the player's own act under a server-authored issuer** — the
`+coop/enrol` shape (`port/coop.rs:1-27`: one command, one batch) and the `CLAIM_ISSUERS` pattern — so
no clerk and no NPC stands on the player's path (D-18-4), and because the fee is a **public E4
contract** with the Registry rather than a counter act it is accepted from anywhere, Vesta included.
2,500 cr for 168 IC days (seven real days, the charter's dormancy week), a three-IC-day grace, one
per account (C23A), bought while Active extending from the current expiry; the contract reaches the
chronicle through the existing `CanonEvidenceBridge` (`composition/econ_canon.rs:62-80`) with no new
code. One instrument in phase 1 because every site is the Registry's; the Republic's instrument for
Phobos is phase 2, and the predicate reads the claim's issuer when there are two.

### 9.2 Blocks — Q-20-2's default

The unit of tenure is the **640 m block** (16 a site), each with its tenure word (§3.1): `commons`
(licence only; a shared reserve worked out by everyone, as a commons should be), `open` (stakeable),
or `held` (an org's). A claim is a `Deposit`-subject `ClaimRecord` over each of the block's
materialised deposits — at most four, so the as-built offer filter (`provider.rs:752-761`, which
honours Deposit claims only) needs no change — Exclusive, priority by filing order, term 168 IC days,
Transferable and pledgeable as titled assets are. **Jurisdiction**: the mining gate honours only
claims whose issuer is the site's issuer, which closes the self-issued-tenure hole
(`provider.rs:841-878`). At most **two live claims per account**. `ClaimRenew` is a new industry op
that extends a live claim's term **in place**, so renewals never spend `MAX_CLAIMS_PER_SUBJECT` 8's
slots (`record.rs:54`; lifetime because the claim index is add-only, `families.rs:69-75`). **Use it
or lose it**: renewal needs **50 t worked in the term** (previewed on `+claim/renew`, shown on
`+registry` as "worked 32 of 50 t") plus the term fee; a claim worked less lapses and the block
reopens, so nobody banks ground for a cycle a week. A lapsed claim goes `AssetLifecycle::Abandoned`
(non-terminal, the last owner kept), and **a stake on a block whose claim is Abandoned takes that
record over** — `TransferTitle`, a new term, holder and discovery stamp — and never files a new one,
so stake, lapse and re-stake never spend the lifetime cap either.

### 9.3 Staking

`+claim/stake` is filed **from the TV standing in the block** (the holder is the actor's own
principal in phase 1; org-held claims are phase 2) and needs your own hand-made Local reading of the
block at R1 or finer — a scan, a sounding or a core; a gift, a purchase or a plan's reading never
qualifies — an Active licence from the site's issuer, and 2,000 cr plus 2,000 a term. Its preview
prints your readings of the block and the public fact "soundings heard on this block before yours:
N", so the contest risk is knowable before the money; `+claim/stake go` files it, the player's own
act under the server-authored issuer, as the licence is; the deposits were materialised at apply, so
nothing is minted here. A second applicant on a block with a live claim **under the site's issuer**
refuses "That block is under claim."; paper under another issuer bars nothing (§9.6). The block turns
`#` on every site map and `+registry <site>` lists it: block, holder, issuer, term, declared family,
discovery stamp, production to date (worked this term against the 50 t floor), contest state — never
truth. The fee contract is public, so a stake and a renewal are chronicle entries; a lapse pays
nothing, so SM.10 commits its own canon draft for it.

### 9.4 Trespass and contest

In phase 1 a licensed rig **will not bite claimed ground**: the mining gate refuses, and the lore is
the Registry interlock (config `[tacsim.surface] registry_interlock = true`), a lever that bounds the
engine — no theft-and-enforcement model until the fight exists. Knowledge is free: a TV may drive onto
and sound claimed ground. Conflict enters by the legal path: **contest by discovery priority** — within
30 IC days of registration a rival with a licence and a hand-made **sounding the site heard** of a
field in the block, stamped *before* the holder's evidence, may `+claim/contest`; the stamps decide,
the claim transfers, the contester pays the stake and the term fee on transfer and the loser's term fee
is refunded (this needs the seize-transfers-tenure fix, `provider.rs:2171-2200`). Because soundings are
heard, a staker knows the count before paying (§9.3). Pledge and transfer of claims are the economy's
as built.

### 9.5 Zones

Each block carries a zone in lane data: `core` for every phase-1 block. Frontier blocks (attackable
without notice, doc 10 5A) arrive with the fight (§17); permanent NPC extractors fall under the
declared-operation regime as doc 10 5A says.

### 9.6 Occator and the Syndicate

Occator holds Occator Rim's inner blocks: two worked by its drones (§12.3) and the rest reserved for
its phase-2 expansion and operator agreements (a 20 % royalty in kind at the cycle). Its **outer
blocks** are open, authored rich (`prospectivity`), and carry **Pallas Syndicate paper claims**, seeded
`Contested { contract }` with their adjudication contract — extending doc 11 §5's one seeded outer
claim to each outer block — the lore reason the Syndicate filed. A licensee may stake one: two issuers
may claim one place and a Contested claim still authorises (`record.rs:864-866`), so both stand and
the player mines. **The Registrar's rule**, deterministic and carried **inside the player's own first
`+claim/renew`** of a papered block, so no Registry actor stands on the path (D-18-4): a block worked
for at least 100 t in the term has the Syndicate paper suspended under the site's issuer and the
player's claim becomes Uncontested; a block worked under 100 t is not renewed and the paper stands.
Worked ground is honoured, with no roll and no Registrar LLM. The authority for it is a
**jurisdiction row on the contest offer**: today `ClaimContest` is offered only to staff or to an
actor who acts for the claim's own issuer (`provider.rs:1539-1555, 2165-2189`); the site's issuer gains
the right to move the contest state of any claim whose subject is a deposit at that site, and nowhere
else, server-authored onto that renewal's offer as the stake is. When doc 11's faction cut lands, a
Syndicate paper refused at Ceres advances a new **Outer Claims** front (proposed here; doc 11 §6 has
none, §18), and registered claim-holders are made Council members in `mush-faction`, which the
`FactionStandingProvider`'s member predicate then reads (`mush-faction/src/standing.rs:33-58`).

---

## 10. Surfaces — D-20-13

Two **pulled grid feeds** on the `atlas.map` shape, as the chart panel is (`port/chart_panel.rs`), so
the one grid printer and the web's grid decoder read them: **`site.map`** — a drive view of 28 by 28
cells (10 m) around the TV at `cell_columns 2` (56 columns) and a whole-site view of 32 by 32 cells
(80 m; a cell shows the best digit of its four pockets) at `cell_columns 1` in the TUI and 2 in line
mode, each block's letter in its corner cell: terrain glyphs from the atlas vocabulary with the three
footings and the crater told apart, `H` the hull, `@` your TV, `o` other units, digits `1`–`3` for your
own R2 readings in the chosen family (`+` where another family also reads), `.` sounded and barren,
`v`/`V` the public pits, `#` a held block's edge; beside it the vector block with the readings list
(`C117 fair metal 120 m NE 25 s`; `C-NW fair–rich metals (5–8 pockets) 300 m N`), the family shown,
the current leg for client interpolation, the hopper, the charge and the lock — and **`body.ground`**
(§3.1). A small pushed `tv.status` keyframe per act change carries the status line, and every
completion prints its line (§1).

**Line mode** is the floor. `+tv/map [family]` prints the drive view windowed on the TV (the family
with the most readings in view by default, named in the block); `+tv/map site [family]` the whole
site; every grid print carries one legend line (`@ you  H hull  o rig  , regolith  ^ rock  = ice
O crater  1–3 ore (t a cycle)  + other family  . barren  v/V pit  # claim`). `GRID_MAX_COLUMNS` and
`GRID_MAX_ROWS` rise from 78 by 20 to **118 by 32** under the 120-column ruling
(`mush-ansi/src/panel.rs:18-22`; `docs/help/tui.md:40-41` still says "drawn for 80 columns" and is
corrected), so the 32-row site view fits in one print. **The TUI**: one lettered screen, **`q`** (free
in both key tables; `j` is the alternative), `MAX_SCREENS` 11 → 12: Left the TV status (32), Main the
map (58), Right the readings and the log (28), 118 at the floor; `z` toggles ground chart and site,
`l` the family layer, `s`/`d`/`x`/`m` prefill scan, drive, sound and mine, `r` re-asks; no mutating
verb on a bare key. **The web**: a `FeedScreen` on the `.site` feed, hotkey `q`, an SVG site picture
and a 2:1 body picture, click-to-prefill. One blessed fixture renders byte-identically on all three.

**Verbs** (every `+tv/…` act previews bare and commits with `go`): `+tv` (status: block, field,
footing, stores, the no-readings hint), `+tv/lease [take|return]`, `/launch`, `/scan` (alias
`/survey`), `/sound [quiet]`, `/drive <addr>|<x,y>|<block>|<bearing> <m>`, `/mine [cycles]` (alias
`/drill`), `/mine stop`, `/recover`, `/unload [qty]`, `/dump`, `/plan <addr>[,…]`, `/plan show|clear`,
`/map [site] [family]`, `/forget <site>|<body>`, `/quiet`; `+coop/refit prospector|standard`;
`+chart/ground <body>` and `+chart/ground map <body>`; `+ship/lift <port>`; `+licence/buy|show`;
`+claim/stake|renew|release|contest|list`; `+registry <site>`. The cockpit's bare selectors (SCAN,
DRIVE, MIND) stay the arena's; `+tv/mine` and `+tv/scan` collide with nothing (the m/mi/min rule
applies to bare selectors only, `match_/cockpit.rs:204-215`). Reads join the `RUN_ALLOWLIST` (both
copies). The help page `docs/help/mining.md` lands its outline — the twelve beats, their verbs and the
`go` rule — with SM.0; `ships.md`, `trade.md`, `coop.md`, `getting-started.md` and `tui.md` gain lines.

---

## 11. The numbers, with their basis

| Number | Value | Basis |
|---|---|---|
| Site | 256 × 256 cells at 10 m, 2.56 km; 16 blocks of 640 m; 4 named fields a block; 256 addressed pockets of 40 m a block; the pad at the centre (1.3 km to any pocket along an axis, 1.8 km straight across) | D-16-5's arena window and `MAX_TILE_CELLS`; a block as the tenure unit keeps a site from being three claims |
| Geology | province one chart cell (6.43°); field 320 m (lean 1–4, fair 5–8, rich 9–16 ore pockets, shifted by an authored `prospectivity`); pocket 40 m (lean 3–6, fair 6–12, rich 12–24 t) | ≤ 64 ore pockets a block by construction, so the cursor sidecar's cap never binds; a site is thousands of tonnes a family against the corridor's 4,000 t season |
| Instruments | scan 48 cells each axis (nine fields), sound 16 (a field), quiet 8; footing sets sharpness | per-axis squares, so "a whole field from its middle" is exact |
| Cycle | 1,800 IC s (75 real s); rock 3,600 | half `PRODUCTION_CADENCE_SECS` 3,600 (`workorder.rs:69`); four to six cycles a hopper keeps a trip under ten real minutes |
| Yield | lean 1, fair 2, rich 3 t a cycle, integer, never rising | goods are whole tonnes (`goods.json:13-15`); D-E7-6 |
| Hopper | 12,000 kg / 13,200 L, `stow.ore` | seven hoppers and 2 t of an eighth fill a Dray-P's 86 t |
| TV speed | tracked 300 cm/s: regolith 2.0 m/s (5 IC s a cell, 21 real s a km), rock 1.0, ice 1.5 | `locomotion.json` tracked row |
| Charge | 100,000 cu; mine 2,000, scan 1,000, sound 2,000, drive 15 cu per cell-cost; bay recharge 50,000 cu per IC h, unloads included | a hold's worth of cycles never drains it; the reserve rule binds long drives and plans |
| Landing | TWR ≥ 1.2 at departure mass and the plan's throttle against `surface_mg`; Dray 206 kN: Ceres 1.79 laden, Luna 0.30, Titan 0.36, Mars 0.13 | `dray.json:83-92`, `passage.rs:76-80`; the ship map's table |
| Hop | `base_s` 3,600 IC s through the laden step (≈ 5,300 out, ≈ 7,000 home with a full hold); burn ceil ≥ 1 t; crew-days by the existing rule; landed 1 crew-day a soul an IC day, floored at one leg home | `passage.rs:235-244, 699, 707-722`; `route.rs:837-846` |
| Dray-P | cargo 86 t / 103.2 m³; a two-slot bay (12 t of TVs); 2.3 t of headroom at the maxima (178,680 kg) | `dray.json:84-92`; never re-rated |
| Tenure | licence 2,500 / 168 IC d + 3 IC d grace, extendable anywhere; lease 1,400 a term + 5,000 deposit; claim 2,000 + 2,000 a term; 50 t worked a term to renew; ≤ 2 live a account; renew or take over in place; contest window 30 IC d | the dormancy week, `lane:2171`; the claim index is add-only |
| Prices | `metal_ore` 70 (5:2 → 175 < 200), `ice_ore` 35 (5:2 → 87.5 < 100), `carbon_ore` 50, `strategic_ore` 600; references 3500 / 1750 / 2500 / 30000 minor | each under the finished good's ask so buy-and-mill never loses; D-18-24 amended for ore |
| Drip | 40 / 40 / 20 / 2 t a good an IC day (960 / 960 / 480 / 48 a real day), standing caps 240 / 240 / 120 / 12 t, within capital | the clerk's 90-day re-post rule; a hold sells in one act; the knob sizes the ore economy |
| Income | one TV on fair ground 4,900 cr gross an active hour (4,300 after the Trust's share), rich 6,700 (5,900), the hop round trip included; a second TV nearly doubles it; one landed hold a plan | §12 |
| Tick | 1 s; visit 64, work 16; one act an operator a fire; ≤ 8 writes a completion, ≤ 3 for any other act | the journey tick; `pool.rs:497-506` |
| Caps | 32 excursions a site; 1,024 world-wide; 4 an operator (2 a Dray-P); ≤ 1 notice a trip, ≤ 1 sounding line a sounder an IC hour; frames < 256 KiB | the Compact's span of control as an engine bound |

---

## 12. Economy

### 12.1 The miner's hour

A **fair** pocket trip with one TV: 300 m out (six real seconds), a scan or a sounding every other
trip, six cycles of 2 t (450 s), home and unload (a minute): nine minutes for 12 t. A **rich** trip:
four cycles of 3 t, six and a half minutes. A Dray-P hold (86 t, seven hoppers and a little) is 65
minutes of fair work or 45 of rich, plus the hop home and back (about nine minutes) and a queued
sale. Per active hour, hops included: fair 70 t, **4,900 cr gross, 4,300 after the Trust's share**;
rich 96 t, **6,700 gross, 5,900 after the share**. Ice at 35/t is 2,450–3,360 gross an hour
(2,100–2,900 after the share): ice is the volume play and the hauling play, not the labour play. Rock
halves the pace; lean ground halves the tonnes. A second TV nearly doubles the tonnes an hour (fair
about 8,800 gross), at a second lease and the attention of two rigs; the hold and the drip's cap
bound it. A full hold of metal ore is 6,020 cr gross at the Exchange, 5,270 after the share.

### 12.2 The miner's day, against the trader

Overheads for one TV: licence 357 cr a real day, claim 286, the lease 200, hops and provisions about
100 for a pilot who lifts home each session and about 340 with the hull on the pad all day (24
crew-days at 10 cr, two legs, two tonnes of burn) — some **950 to 1,200 a day**. A miner at an hour
and a half a day grosses 7,350 on fair ground and 10,050 on rich, pays the Trust 920–1,260, and nets
about **5,200 to 7,600** after the overheads; the trader nets about 5,390 outbound and 7,740 home
after its own share, on a passage of about twenty real hours with half an hour of play (doc 18 §4.1,
constant sky, coast law; about 18 h and a heavier burn under the flip law, doc 19 §4.3). The miner's
90,000 buyout lands about **day 17 on fair ground and day 12 on rich** at an hour and a half a day,
**day 29 and day 19** at an hour; the trader's about day 13 (day 15 with doc 18's 20,000 float). An
offline fill plan is one landed hold — 5,270 cr after the share for a player who logs in once to
file the plan and the sell order, the trader's order of money — and nothing more, because the plan
never re-arms. Mining is the labour career: little capital, active hours, the trader's money at an
hour and a half a day on fair ground and more on rich or with two rigs, and the rich blocks — finite,
staked, contested — are where it is won.

### 12.3 Stock, prices and the NPC side

Mined ore enters the finite economy only at Ceres, only through funded intakes (7A: Occator's
capital raised by tranche as lane data; the Exchange's own), and leaves it only through the mill
into asks the trade run already sells from. Eight charter hulls at half a leg a day draw about 80 t of
concentrate and 320 t of water; one fair miner's 70 t of ore an hour is 28 t of concentrate, so
**three miner-hours replace the Exchange's daily draw** and the drip caps the rest at 960 t of each
bulk ore a real day. Prices are authored plus the reach premium as today; ore never moves an authored
price. **Occator's drones** are NPC excursions on its two inner blocks under its own licence and
claims, run by a dispatcher principal on the same tick (bounded to four excursions, in-kind supply
through its own mill, never a cash mint), so "the corridor's deposit and extractor drones" of doc 11
§4.1 become supply the ledger can see, and Occator's 9,600 t water seed lasts a season rather than a
month. The Pallas Syndicate mines nothing on the corridor; its paper is pressure.

**Levers, all data**: the ore bids, the drip and its caps, the mill ratios, the hopper, the cycle, the
yield table, the field and pocket distributions, the authored prospectivity and worked shares, the
fees and the worked floor, the lease, the references for the share.

---

## 13. Cuts

Each cut is one commit on `main` with the Fable trailer, pushed when green, with a `## SM.n` heading
in **`docs/design/20-cut-notes.md`** (its own file; Q-20-13). Sizes as doc 18 §5: S a day, M a few
days, L a week or more. One lane version per cut, as the A-cuts did. No deploy without the owner.

| Cut | What | Size | Depends on | Acceptance |
|---|---|---|---|---|
| **SM.0** | This document; the README row (and the 19 and 20 rows moved into numeric order); `20-cut-notes.md` started; `docs/help/mining.md`'s outline (the twelve beats, their verbs, the `go` rule, landing at the lettered sites) | S | — | every Q-20-n carries a default; the README rows in numeric order |
| **SM.1 the ground** | `tacsim::surface::geology` (HMAC over the fourth key family, the province/field/pocket passes with the authored `prospectivity` shift, the reading rule with a version); the geology key family in `mush-config` and its fingerprint record; `@geology`; the off-loop site composite LRU and geology table; the `i16` validator rule on `datum ± relief` and the symmetric cast | M | SM.0 | a fixed test key gives pinned digests of a 256 × 256 site; a property test over 10⁶ lattice points shows every field's word is the honest summary of its pockets and ≤ 64 ore pockets a block; a wrong fingerprint disables the layer with the reason on the readiness line; `provider_rng_policy` green |
| **SM.2 sites and landing** | lane **v8**: `BodyDef.surface_mg/radius_km/geology` and the Ceres and Vesta geology tables, `LocationDef.lat/lon`, the `site` block (per-block tenure, `prospectivity`, `worked_permille`, zone, issuer) with the zero-orgs rule and the serde defaults, the `freeport_registry` org and treasury, five Ceres sites and Rheasilvia Commons with routes; atlas region packs (recipe, the pad room at the centre with `port.dock`, craters); the sixteen block place Things a site; the landing law at the plan's throttle; the hop leg with the ascent term and the 1 t floor; landed life support with the one-leg-home floor and the stores line; the berth and arrival lines; `+ship/lift` docked or settled; recall counts hops; `+chart` lists sites with their survey words | M | SM.1 | `+ship/plan ahuna` with a full tank, one TV aboard, a 120 crew-day locker and one soul plans ≈ 5,300 IC s and 1 t, frozen in the ticket; a Luna site refuses by TWR at planning; arrival settles on the pad and `+ship/lift ceres` works; a hull landed ten days still lifts; a week of hops trips no recall; the two old routes' pins unmoved; sixteen place objects a site; land-and-leave survives a restart |
| **SM.3 the TV and the tick** | `UnitClass::Vehicle`; `LogisticsDef.drive` optional; the tools content kind; `tv_prospector.json` and `dray_p.json` (a two-slot bay); lay-up (TV Thing, unit, cab, hopper store, the `tacsim.vehicles` vault); `payload_kg` counts the bay; the two excursion records; the `tacsim.surface` tick; `+coop/refit`, `+tv/lease [take|return]`, `+tv`, `+tv/launch` (and the implied launch), `+tv/recover` (the placement half), `+tv/dump`; `TV_OUT` and `HOPPER_LOADED`; the `go` rule | L | SM.2 | a Dray-P with two TVs aboard plans the same figures as a Dray at equal mass; a lease refused without a licence or a free slot and a return refused with the TV out; launch refused under way and at a port; recover from two cells refused; a restart mid-launch completes at the due instant; `live_endpoints` never lists a hopper for any player; departure refused with a TV out; a bare `+tv/<act>` commits nothing |
| **SM.4 knowing and driving** | the chart documents (coverage, `ChartProvenance`, `+tv/forget`); `+tv/scan` (nine fields, sharpness by footing) and `+tv/sound [quiet]` (a field) as timed acts with the site-wide notice and its budget, `+tv/quiet`; `+tv/drive` with every target kind (A* over the composite at the site budget, the frozen leg, IC timing, charge, the reserve rule); the `site.map` feed, `+tv/map` with the legend line and block letters, the completion lines; the grid caps raised to 118 by 32; fixtures | M | SM.3 | readings equal the quantiser for covered cells only; a second principal sees no ore glyph; a repeat from the same cell is refused byte-identically at no cost; a sounding reaches other parties as a bearing and band and nothing more, a quiet one reaches nobody, a tenth sounding in an IC hour prints no line; a sounding from a field's middle reads all 64 pockets and a scan from a field's middle reads all nine fields; a regolith scan prints bands that contain the truth; 1 km of regolith arrives at exactly 500 IC s and 2,250 cu; an act that would strand the TV is refused; the arrival line prints over SSH; the observer-rule witness over SSH and web |
| **SM.5 legal and market plumbing** | `with_extraction_licence` wired to `tacsim.licence.freeport`; lane **v9**: the Ceres ore bids with their drips and caps and Occator's tranche, the ore `reference_minor` rows; `+licence/buy|show` as the player's act under the server-authored issuer, the fee a public contract accepted anywhere, extension while Active; goods v2 (four ore rows, `stow.ore`; the Dray-P accepts it); the drip step on the stock arm with its own predicate; the fixed-reference variant with its validator rule; the sell preview's unfilled line; readiness lines | M | SM.2 | a licence makes `extraction_authorized` answer Ok over a staff-seeded claim; the lapse at IC day 172 refuses at the next act; a buy while Active extends the expiry; 10 t of ore staff-placed in a hold sells at the bid with the Trust's row at a quarter of the upper half; a filled intake posts afresh at `min(cap, unfilled + drip)` within the hour and never past capital, and an 86 t hold sells in one act against the cap; a fixed reference for a good some port asks is refused at load; the market-never-mints probe green |
| **SM.6 the industry seams** | `IndustryHandleCell` (read access for composed providers); `with_system_opener` and the site applier's materialisation with receipts over the block places (accessibility 100, the one-point curve, the ore good as commodity, `worked_permille` pre-drawn); `OreDeposit.external`; the `ExternalDraws` trait and the sweep invariant; `ClaimRenew` in place and the Abandoned take-over; the seize-transfers-tenure fix; the contest offer's jurisdiction row; the pocket cursor sidecar as the lock's home | M | SM.1, SM.5 | two boots with the same key open identical reserves; each block's reserve equals the sum of its pockets less the worked share; a re-apply credits nothing; a site opens 64 deposits over 16 places and never attempts a ninth on one; `@extract/start` refuses a surface block; a forged pool debit quarantines the block; renewing nine times never reaches `MAX_CLAIMS_PER_SUBJECT`; a block staked, lapsed and re-staked ten times still accepts a stake; the site's issuer may suspend a foreign claim at its site and nowhere else; every E7 industry suite green |
| **SM.7 mining** | `+tv/mine [cycles]` / `+tv/drill` and `stop`; the start's gates and the preview from the operator's own row (the one-cycle run on an unread pocket); the pocket lock on the sidecar; the completion bundle (transfer of the ore good, cursor, receipt) and the tick's log, chart row and line; the `tacsim.surface.mined` ring; terrain-scaled duration; pits on the map | L | SM.4, SM.6 | a fair 7 t pocket yields 2, 2, 2, 1 then "worked out"; `initial − pool == Σ drawn` after a 10,000-cycle soak; the hopper never exceeds 12 t, a named run that does not fit refuses and names the count that fits, and the bare form stops at the hopper; the preview on an unread pocket prints no class and commits one cycle; `stop` lets the running cycle yield; a replayed completion is a HIT; a second rig on the pocket refuses with the public line; no licence, a lapsed licence and another's block each refuse with their public line; pits render for everyone; the cycle line prints over SSH |
| **SM.8 unload, sell, the story** | the unload half of `+tv/recover` and `+tv/unload [qty]` with `max_more`; the hold plan generalised to any stowage block while `governed_hold` stays programme-hull-only; the story test on `build_engine` at rate 24 (kit up with all three desk verbs, plan, lift, scan, sound, mine to full, recover, lift home, sell; a restart inside); `docs/help/mining.md` complete; the help tripwires | M | SM.7 | the story is green on the production composition with one staff act (the rate); unload refuses under way and with the TV out; the hopper is refused at sale by `HOLD_ONLY_SOURCE` and by `may_take`; T-45 and the legacy-handler tripwires green |
| **SM.9 the quarry screen and the ground chart** | `body.ground` (`+chart/ground`, `+chart/ground map`; `+chart/body` untouched); TUI screen `q` (`MAX_SCREENS` 12); the web `FeedScreen` and pictures; keys; `RUN_ALLOWLIST`; the boot-parity manifest; fixtures | M | SM.4 | one fixture renders byte-identically in line mode, the TUI and the web at 120 columns; `+chart/body ceres` still prints the elements; the wash is identical for two viewers and a best reading appears only to its viewer; screen-count tests updated; no mutating verb on a bare key |
| **SM.10 claims** | `+claim/stake|renew|release|contest|list`, `+registry`; Deposit-subject claims over a block's deposits as the player's act under the site's issuer; the hand-made evidence rule and the stake preview's heard-soundings line; the 50 t worked floor, lapse to `Abandoned` with its canon draft, the take-over on re-stake; the interlock lever; discovery-priority contest on a heard sounding with the contester's fees; the seeded Occator holdings and Syndicate paper with their adjudication contract; the Registrar's worked-claim rule inside the renew act; the two-claim cap; claims on the site map | L | SM.7, SM.9 | staking with a gifted, bought or plan-made reading or none is refused and a hand-made core suffices; a second stake on a block held under the site's issuer refuses and a Syndicate paper bars nothing; a self-issued claim from a player org authorises nothing at a Registry site; a claim worked one cycle is not renewed and the block reopens; a Syndicate block worked 100 t shows the paper suspended after the player's renewal, one worked less is not renewed, and no Registry actor submits anything; the stake reaches the chronicle through the bridge and the lapse through its draft |
| **SM.11 plans and offline** | `+tv/plan <addr>[,…]` with the block rule, `show`, `clear` (the compiled vocabulary, one landed hold, 288 IC h, knowledge-gated, no re-arm); `await tv` to 288 IC h; the `depart` refusal; idle return-and-park; the notice budget; span of control 4; `via: Plan` on plan-made readings; the offline story across a restart | M | SM.8, SM.10 | a logged-off player's plan fills the hold and parks, the queue lifts, flies and sells, the notices read on reconnect; the plan does not re-arm; killing the server midway changes nothing in the final post-image; a fifth excursion for one operator refuses; a plan-made reading does not qualify a stake |
| **SM.12 the mill, the drones, the cost lane** | the Exchange's and Occator's mill arms with receipts and `restock_ask`; Occator's dispatcher excursions on Occator Rim; a harness cohort (64 TVs over 8 sites, every act cycling; tick p99, writes an act, composite churn; the Vesta passage included) beside the arena baseline; a `mush-bot` cohort playing the whole loop over SSH | L | SM.11 | over a simulated IC month the Exchange's concentrate ask rises only by milled ore with conservation asserted; NPC excursions never exceed their caps nor sell into player intakes; tick p99 < 5 ms a fire, ≤ 8 writes a completion and ≤ 3 for any other act; the bot cohort completes the loop in line mode (doc 14 P10) |

**Phase 2** (own cuts, after the owner's rulings): the second jurisdiction (Phobos under a Republic
lands office at Stickney with its own licence instrument and a Stickney ore bid; a new `microgravity`
region tag the locomotion table prices); survey trade and commissions (chart rows as
`Representation::Entitlement` listings through a `DeliveryProvider`; Occator survey commissions settled
on `tacsim.survey.logged`); implicit landing anywhere on the lattice; the crewed cab if Q-20-4 goes
that way; org-held claims (`+claim/stake as=<org>`) when players can hold orgs; Occator's expansion
and operator agreements.

**Phase 3** (§17): asteroid-field drones; frontier blocks and the fight.

---

## 14. Decisions taken here

| # | Decision | Where |
|---|---|---|
| D-20-1 | The surface loop runs world-side on the IC clock, every act with an IC due instant applied by the 1 s `tacsim.surface` pull tick; never the arena, never an atlas window, never a per-excursion schedule for players. The arena hosts fights, not chores | §2 |
| D-20-2 | A site is a vendor-less port entry with a `site` block over a 256 × 256 atlas region at 10 m (the arena window), 16 blocks of 640 m with per-block tenure, authored prospectivity and worked share, a place object each, four named fields and 256 addressed pockets a block, the pad at the centre; phase 1 is five Ceres sites and a Vesta commons with its own geology | §3.1 |
| D-20-3 | The landing law: TWR ≥ 1.2 at departure mass and the plan's throttle against `BodyDef.surface_mg`; a hop's `base_s` 3,600 IC s through the laden step with an ascent term ceiled to a tonne; crew-days by the existing rule and an IC day landed, floored at one leg home and printed | §3.3 |
| D-20-4 | Geology is a pure HMAC-keyed function under a fourth durable key family with a fingerprint pinned in the world; province, field, pocket in one word set; the four raw families; readings nest; no RNG | §3.2 |
| D-20-5 | One conserved `OreDeposit` pool per (block, family) in the family's ore good, credited once at site apply by a composed system opener from the applier's actor; a bounded pocket cursor sidecar that is the lock's home; every cycle a pool transfer; the invariant through a composed `ExternalDraws` trait on the reconcile sweep; `OreDeposit.external`; a pocket's class fixed, its remainder shown by preview and pits | §3.4 |
| D-20-6 | The TV is a full `UnitSpec` of a new unfieldable `Vehicle` class with stowage-only logistics and a two-room interior, teleoperated in phase 1; tools are a content kind with per-axis radii; charge bounds a run, with a reserve rule | §4.1 |
| D-20-7 | The Dray-P carries up to two TVs in a bay counted in `payload_kg` under the 181 t rail, never a re-rate; `+coop/refit` swaps it; `+tv/lease` is the player's act under Occator as lessor; `TV_OUT` and `HOPPER_LOADED` refuse departure; a hopper vault is the system principal's | §4.2–4.3 |
| D-20-8 | Knowledge is coverage plus its own provenance enum per principal, re-derived at render; R0 public, R1 the nine-field scan sharpened by footing, R2 the field sounding heard site-wide with a quiet form; only a hand-made reading counts as discovery and only a heard sounding as contest evidence; fog lines byte-identical | §5 |
| D-20-9 | Every `+tv/…` act previews bare and commits with `go`; `+tv/mine` previews from the operator's own row (one cycle on an unread pocket), commits a run bounded by the pocket and the hopper, refuses a named count that does not fit and names the one that does, never clamps; yield by pocket class, never rising; a pocket lock; knowledge acts legal during a run; no `until=full` across pockets for a seated pilot | §6 |
| D-20-10 | Grade is spent at the drill as tonnes; mined matter is raw ore goods under `stow.ore` bought only at Ceres; the hopper never sells; the capacity plan, not the sale predicate, is generalised; the mill is P5's supply half; intakes refresh by a drip step of their own to a standing cap; the Co-op's reference for ore is a fixed minor-unit price at half the bid, amending D-18-24 for goods no port asks | §7 |
| D-20-11 | A fill plan is one landed hold, 288 IC hours, a closed knowledge-gated vocabulary compiled from a pocket and block list, one a TV, never re-armed; `await tv` to 288 IC h; four excursions an operator, two a Dray-P; idle return after 24 IC h; Q-18-22 extended and T13 read as owned-asset production and the character's own instruments on a dispatched sortie, plan-made readings never discovery | §8 |
| D-20-12 | One licence instrument per issuer, bought and extended anywhere as a public contract; blocks as Deposit-subject claims filed by the player's own act under a server-authored issuer; renew in place and take over an Abandoned record on re-stake; 50 t worked a term to renew; two live a account; contest by a heard sounding's priority with the contester's fees; jurisdiction is the site's issuer, on the mining gate, the staking refusal and the contest offer, carried inside the player's own acts; the interlock in phase 1 | §9 |
| D-20-13 | Two pulled grid feeds on the `atlas.map` shape (`site.map`, `body.ground`; `+chart/body` untouched); `+tv/…` verbs with `scan` and `mine` as the words and `go` as the commit; screen `q`; line-mode grids 118 by 32 with a legend line; every completion a printed line | §10 |
| D-20-14 | Phase 1 is Ceres and a Vesta commons, every block core (amending doc 11 §5's margins), no surface combat; D-16-3 stands; promotion is phase 3 behind an IC-crossing ruling | §3.1, §17 |

---

## 15. Questions for the owner (defaults taken, none blocking)

- **Q-20-1 What the drill produces.** Default: raw ore goods milled at Ceres (§7.3). Alternative: the
  existing concentrate and water, grade folded into tonnes — simpler, but it makes mine-then-haul the
  dominant play on every charter and bypasses the Exchange's purchase price. This also supplies P5's
  restock half and so touches Q-18-19's "no top-ups", which is why it is the owner's.
- **Q-20-2 Tenure.** Default: 640 m blocks with per-block tenure, commons on every body, hand-made
  own-reading evidence to stake, 50 t worked a term to renew, two live claims a account. Alternatives:
  whole-site claims (three stakes lock Ceres), shared per-pilot permits, a frontier free-for-all with
  tenure only for permanent rigs.
- **Q-20-3 The offline bound.** Default: one landed hold a plan, 288 IC hours, never re-armed, with
  `await tv`. Alternatives: live-only mining; at most eight closed steps and no launch; a plan that may
  cycle to N holds (re-rules Q-18-22 and risks the economy).
- **Q-20-4 Crewed cab or teleoperation.** Default: teleoperated from the settled hull. The crewed cab
  — a body in the cab, exposure, rescue, the Starflight feel — stays open and is a later cut on the
  same TV.
- **Q-20-5 The Co-op's share on mined ore.** Default: a fixed programme reference at half the bid,
  amending D-18-24 for goods no port asks. Alternatives: as built (250 ‰ over the goods table's
  `reference_major`, the logged baseline path; zero only if that is authored at the bid); a flat
  royalty a tonne by a separate contract; exempt.
- **Q-20-6 Where the TV comes from, and how many.** Default: Occator leases them (`+tv/lease`, 1,400 a
  term, 5,000 deposit), the Co-op refits the charter onto a Dray-P (`+coop/refit`) with a two-slot bay.
  Alternatives: one slot (a 75 s rhythm and nothing between cycles); the Co-op supplies the TV with the
  refit; a purchase.
- **Q-20-7 Landed life support.** Default: a crew-day a soul an IC day while settled on a site,
  floored at the stores one leg home needs and printed on arrival (a solo week needs a resupply).
  Alternative: none while landed, as today.
- **Q-20-8 The words.** Default `+tv/mine` (alias `drill`), `+tv/scan` (alias `survey`), and `go` as
  the commit word on every `+tv/…` act.
- **Q-20-9 Soundings observable to rivals.** Default: yes, as a bearing and a range band (one line a
  sounder an IC hour), with a quiet half-radius sounding nobody hears.
- **Q-20-10 The second body in phase 1.** Default: a Vesta commons under the Freeport Registry
  (Rheasilvia Commons) with richer metals and the strategic shelf, reached by a seasonal passage, so
  "choose a planet" is a real choice from the first cut. Alternatives: Ceres alone; Phobos first (needs
  a Republic issuer and a Stickney ore bid, phase 2's shape).
- **Q-20-11 Prices, the drip and its caps.** Default: `metal_ore` 70, `ice_ore` 35, `carbon_ore` 50,
  `strategic_ore` 600; drips 40 / 40 / 20 / 2 t a good an IC day to standing caps of 240 / 240 / 120 /
  12 t. Lane data.
- **Q-20-12 The key.** Default: a fourth durable key family on disk with a fingerprint record and a
  loud disable on mismatch. Alternative: a config seed (silently moves every reserve if it changes).
- **Q-20-13 Cut notes.** Default: `docs/design/20-cut-notes.md`, its own file.

---

## 16. Risks, and what this document refuses

**Risks.** The surface tick shares the engine's 64-actor submit queue with the port tick; SM.12's
cost lane measures both under load before any site goes live. The composite LRU can spike CPU when
many sites are active; builds stay off-loop and the lane bounds them. Interpolated TV positions mean
a client and the server disagree about an intermediate pose for up to one fire; harmless while
nothing interacts mid-leg, and the promotion phase must derive the true pose from the leg record.
Lane v8 and v9, goods v2, the Dray-P, a new unit class, a new content kind, two new feeds, a new
screen and the grid caps all move pins; each cut re-blesses, and a kept world needs an operator plan
and apply. The geology key is a new operational secret: it is backed up with its key directory, and a
lost key is a disabled layer, not a corrupted one. Tenure fees are minutes of mining; the worked floor
and the finite rich pockets are the conflict, not the fees. A second TV nearly doubles a pilot's
tonnes; the drip's cap, the hold and the lease are the levers if that proves too strong. Occator's
drones and the drip set the ore economy's size; both are knobs the owner tunes on dev1. The Registry
interlock is a lore shortcut that the fight will have to replace with enforcement play. The Vesta
commons is the first site reached by a passage rather than a hop: its hull is away for days, which
the queue's `await tv`, the plan's 288-hour bound and the licence's anywhere-renewal were sized for,
and the cost lane must include it.

**Refuses.** No RNG anywhere: variety is place, uncertainty is time. No auto-fire and no repeating
loops for a seated pilot. No world-changing bare keys. No arena, window or snapshot on the surface
loop in phase 1. No landing on Mars, Luna or Titan. No grade-priced bids and no graded pools. No
player-owned TV vault. No per-pocket deposit. No cell edits and no depletion features. No new terrain
class and no eleventh commodity family. No surface that reveals truth, the mine preview included. No
claim-jumping by drill in phase 1. No surface combat and no frontier blocks until promotion exists. No
mining-only, mind-only or pilot-only verbs. No deploy without the owner.

---

## 17. The drone phase and the fight (reserved, not built)

**Asteroid fields.** A `field` ground kind on the site block: an orbital site whose 256 × 256 grid is
an occupancy layer of rocks from the same geology (a rock cell is a pocket; empty cells are void) at
a lane body or a belt station such as Kirkwood. Locomotion `thrust` with a new `microgravity` tag
added to the locomotion table (phase 2, §13). **Drone bays** are a compartment with `maximum_items` N
— the leash, a lore lever: a Dray-P runs at most four drones, span of control as an engine bound.
Drone units in the same tools content kind (a grapple-drill, a 2 t hopper, 1 t a cycle). Repetition
arrives only through doc 17: the plan's steps become station functions (`scan()`, `sound()`, `goto()`,
`mine()`, `recover()`) in one D-UM-47 cut with a `mining` role admitted by the metadata gate, a stock
mining doctrine and presets, and a twin fixture; each drone sortie is sealed from bay to bay under the
`ExcursionId` this document mints. Doc 15's light lag becomes the order latency between a port and a
remote field.

**The fight.** A frontier block (the Pallas outer claims) where a hostile arrival **promotes the site's
ground into an arena** — the same 256 × 256 grid by digest — behind an IC-crossing ruling that
succeeds D-16-3 and D-UM-50 (what crosses at check-out, what returns at check-in, how a snapshot
rewind cannot double-mint). The TV's class becomes fieldable, `ComponentRole::Tool` and
`DueKind::DrillComplete` mirror the weapon's `ready_at` pattern (`arena/weapons.rs:991-1211`), the
hopper stays world-side under T2, and the excursion `Holds` while the arena holds the TV. Then a TV left
out is a choice, return-on-idle is a survival rule, and claim-jumping is a scene. Nothing in phase 1 is
reworked for it: the site extent, the sortie identity, the tool shape and the knowledge rows are built
to be promoted.

---

## 18. Amendments owed (made by the cuts that land them)

- **README**: the doc 20 row, and the doc 19 and 20 rows moved into numeric order after 18a (SM.0);
  rows for `09a/09b/09c`, `11-tacsim-setting`, `16c`, `17e`, `17f` and `18-cut-notes` are missing today
  (SM.0, a drive-by).
- **Doc 10 §5** — the loop gains its surface arm: scan → sound → stake → mine → haul → mill.
- **10b §3.3 / §15** — the cut-3 handoffs this document builds (a cadence, a system opener, read
  handles, `ClaimRenew` and the Abandoned take-over, `OreDeposit.external`, the `ExternalDraws` trait,
  the contest offer's jurisdiction row, the pocket sidecar) recorded as E7 residue closed (SM.6).
- **Doc 11 §4.1, §4.12, §5, §6, §7** — Occator's terrain-vehicle lease beside its drone lease and its
  drone blocks, the Exchange's and Occator's ore intakes, the Freeport Registry as an org with a
  treasury, the Occator claims' margins ruled core until the fight, the Syndicate paper on every outer
  block, the proposed Outer Claims front, Vesta's commons under the Registry (SM.2, SM.5, SM.10).
- **Doc 17 §12, §22** — the sortie seal at a bay; the `mining` role reserved (phase 3).
- **Doc 18 §4.1 / D-18-24, §4.2, §4.4, §5, §8** — the fixed-reference variant for goods no port asks
  (SM.5); the Dray-P beside the Dray; `await tv` and `+ship/lift`; P5's supply half lands here (the
  mill, SM.12); Q-18-22 extended by D-20-11.
- **Doc 19 §3** — `BodyDef.surface_mg`, `radius_km`, `geology`; `LocationDef.lat/lon`; sites as
  chart destinations with their survey words; `+chart/ground` beside `+chart/body` (SM.2, SM.9).
- **Doc 14** — a region placed on its body by `lat/lon` (`mush-atlas/src/apply.rs:427-447` forces
  `parent None` today); the `i16` validator rule and cast; the grid caps (SM.1, SM.4).
- **Doc 16** — nothing changes; §12 gains a note that D-20-1 keeps the arena to fights and that the
  promotion phase will bring the IC-crossing ruling.
- **`docs/help/tui.md`** — the lettered screens `c` and `q`; "drawn for 80 columns" corrected.
