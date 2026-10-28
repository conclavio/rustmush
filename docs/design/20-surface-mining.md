# 20. Surface mining — the quarry loop: a terrain vehicle on a body's ground

**Draft 0.15 — "as built" through SM.12c-lane (the whole of SM.12: the train complete) and
the post-train RK-1 defect cut (2026-10-26).** Nineteen build cuts have landed on
`main` after SM.0 (the document, `11c50e84` and `f3df3205`): SM.1 `61324cba` (with its follow-up
`6f797974`), SM.2 `bd718fb2`, SM.3a `96cc7303`, SM.3b `eb99c9be`, SM.5 `64cd1f8e`, SM.4a
`354832bf`, SM.6 `70c2cd12`, SM.4b `d367bf3b`, SM.7 `dd48ed45`, AP `74c5801f` (the atlas path
search's cost, an engine cut with no §13 row of its own, owed by SM.4b), SM.8 `9922c9c7`, SM.9
`16b0ca4e`, SM.10 `fed09da2`, SM.11 `09854d6a`, SM.11r `8cc0aa64` (two SM.11 defects from the
owner's external review, doc 20a §2's row, no §13 row here), SM.12a `9df93246` (the mill), SM.12b
`1b5dd371` (Occator's drones, with the lead's follow-up `f171b36c`), SM.12c-bot `d2f4ee7c` (the
bot cohort, the lead's by-hand cohort run on it recorded in `d842fdd0`) and SM.12c-lane `389383f1`
(the cost lane, the train's last cut) (SM.5 was built before SM.4a by the lead's ruling; SM.6 then
landed before SM.4b; with SM.12c-lane every row of §13 has landed and the surface-mining train is
complete as built; §13; §18's "Landed").
Each cut's record — what landed, what it deferred and why, the lead's rulings and the defaults it
took for the owner — is its section of [20-cut-notes.md](20-cut-notes.md), and this draft folds the
"Doc 20 amendments owed" of those sections in as **"As built (SM.*n*):"** notes (AP's as
**"As built (AP):"**) beside the text they amend, in doc 18's manner: the Draft 0.4 sentence is kept
and the note says what the tree does, or a stale label (a lane or goods version, a role word) is
corrected in place. Draft 0.5 folded the first four cuts, Draft 0.6 SM.5's and SM.4a's, Draft 0.7
SM.6's and SM.4b's, Draft 0.8 SM.7's, Draft 0.9 AP's and SM.8's, Draft 0.10 SM.9's, Draft 0.11
SM.10's, Draft 0.12 SM.11's (SM.11r's own commit added its notes), Draft 0.13 SM.12a's, SM.12b's
and SM.12c-bot's, Draft 0.14 SM.12c-lane's, and Draft 0.15 folds the RK-1 cut's (doc 20b §6, its
record the cut notes' last section, as **"As built (RK-1):"**) and the owner's rulings of
2026-10-26 on Q-20-10, Q-20-12 and the provisional rulings (§15); where
a cut's fix round changed what its judge's amendment said, the note carries the round's truth and
says the round did it. A note that
records a lead ruling or an owner-critical default says so, and each such default stands as the
lead's until the owner rules, as the Q-20 defaults did (§15); a default the owner took
provisionally on 2026-10-24 (O-1, O-5, O-7b; then O-8, O-9, O-10, O-14, O-19 and O-25 to O-28, doc
20a §8) is said to be the owner's provisional ruling, not the lead's (§15). Draft 0.4's file:line
cites stand as that draft read them; a note's cites are its cut's.

**Draft 0.4 — 2026-10-19** (Drafts 0.1 to 0.3 the same day, each folded after a verification pass:
119 findings, then 60; 0.4 folds the owner's review of eight findings, §19). The owner's ask,
verbatim: *"once upon a time there was this game called Starflight ... you had a ship, you had a
Terrain Vehicle (TV), you flew the ship between star systems that had planets, you chose a planet,
you got a mercatur projection map of the planets surface, you landed your TV on it, you drove around
and mined minerals, you could scan to find the deposits, drive over to one and hit mine and you'd get
stuff depending on where you were.. then when the TV storage was full you went back up to the ship to
offload and repeat .. that's one of the gameplay loops i want to support, you fly to an asteroid,
moon, planet and then you mine it .. surface mining via a vehicle, or potentially drone-based mining
of asteroid fields, but we'll start with the surface version .. help me flesh out what this whole
mining system could/should look like as a design plan"*.

The end state is already on record, in the owner's ruling R-B of doc 17: *"a lance may go out to mine
(one armed escort and AI miners) ... large IC region maps where units go out, mine, haul, fight and
build, return to base, empty their cargo, sell or refine."* This document is the first phase of that:
the hand-driven surface loop on Ceres and one outer commons, built on what the tree has, with the
drone phase and the fight reserved by shape rather than built. It was written from five independent
designs (player's seat, engine reuse, world and economy, async and scale, knowledge first), fifteen
judgements and a completeness pass over the tree at `4465bf06`, then verified twice against the tree
and reviewed by the owner; the designs, judgements and findings are kept in the session's plan data,
not in the repository, and every file:line below was read this pass.

Decisions are **D-20-n**, inline where they are taken and listed in §14; the owner's open questions
are **Q-20-n** (§15), every default of which the owner accepted on 2026-10-19. Cuts are **SM.n**
(§13). No deploy without the owner.

---

## 1. The loop, from the player's seat

At world rate 24 one real hour is one IC day (`crates/tacsim/src/port/readiness.rs:37`), so every
beat below is an IC duration that the rate turns into the right real pace. Every beat is a previewed,
committed act with a due instant; none is a trigger to mash (the owner's rule of 2026-10-02: support
the decision, not the trigger). **One rule for every `+tv/…` act**: `+tv/<act> …` prints the preview
and commits nothing; `+tv/<act> … go` commits exactly what the preview printed. With two rigs aboard a
leading slot number names the rig (`+tv/2 drive C117 go`); without one the verb addresses the rig
last addressed. Every act's completion prints one line to the operator in line mode ("Arrived at
C117.", "Rig 1, cycle 2 of 4: 2 t metal ore; hopper 4 of 12 t."), the panel keyframe beside it.

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
   **As built (SM.3a):** Occator's counter is the Ceres Exchange Hall (the lead's default, standing
   until the owner rules); `+coop/refit` also refuses with provisions aboard, with a rig in the bay (or
   a live lease record) and away from the desk; the term is re-debited at the pilot's next `+tv/lease`
   act, not at the next dock (§4.3). `+licence/buy` is SM.5's.
   **As built (SM.5):** `+licence/buy` previews and `+licence/buy go` buys, the pilot's own act
   under the server-authored issuer `freeport_registry`, accepted from anywhere; "bought while
   Active" reads "while in force or in its grace", and such a buy extends from the expiry (§9.1).
   `+tv/lease` is refused without a licence in force or in its grace (§4.3). The desk's `look`
   listing the verbs is not built (SM.3a's residual R4), and no cut's row owns it.
2. **Pick a body, then a site** (one to two real minutes). `+chart` lists sites as destinations with
   their passage now and the Registry's survey word for each ("Rheasilvia: metals fair–rich, strategic
   lean (survey); passage 2 d 4 h; licence 4 d left"): Ceres's by a hop of minutes, Vesta's commons by
   a passage of days whose length the season sets, to richer ground. `+chart/ground map ceres` draws
   the body as an equirectangular grid with that survey as a wash, the port and the sites by letter;
   `+chart/ground ceres` lists the sites with their survey words first, their tenure (commons, open
   blocks, held blocks) and the viewer's own best reading at each. Decision: the worked-over commons
   next door, fresh open ground farther off, your own block, or Vesta when the sky favours it.
   **As built (SM.2):** `+chart` prints each site as a destination, then a "Registry survey: …" line
   and a "Tenure: …" line under it (`SURVEY_LINE`, `TENURE_LINE`); the survey reads "unavailable"
   without a live geology key. The single-line form above is not what shipped; `+chart/ground` is
   SM.9's.
   **As built (SM.5):** the site line's "licence 4 d left" is not built, and no cut's row owns it;
   `+licence` (or `+licence/show`) says whether the licence is in force, in its grace or lapsed.
   **As built (SM.9):** `+chart/ground [<body>]` lists the body's sites — the survey words, the
   tenure, the passage planned at the hull's load and the viewer's own best reading — and
   `+chart/ground map [<body>] [<family>]` draws the body with one family's wash, the ports and the
   sites by letter (§3.1). The site the hull stands at reads ", where you are." with no passage; from
   a site's pad every other site reads "no passage from <site>", since site ports route only to the
   Freeport (a question the owner holds, §15).
   **As built (SM.10):** the "Tenure: …" line and `+chart/ground`'s tenure read the block records
   (`tacsim.claims`), counting live claims with the word "{n} claimed" after the open blocks and
   before the held groups (the fixer's reading of lead note 0a; the word is OC10, an owner-critical
   default the lead accepted, standing until the owner rules).
3. **Hop and settle** (four to five real minutes). `+ship/plan ahuna`, then `+ship/lift ahuna` (an
   alias of `+ship/undock`, docked or settled) files the hop like any passage; it burns a tonne and a
   crew-day a soul. Meanwhile `+registry ahuna` and the ground chart; the arrival line calls the pilot
   back: "The Dray-P settles onto the pad at Ahuna Flank. Stores 96 crew-days: 3 d 22 h here before
   short rations." Decision, before lifting: sell what is in the hold now, or fill it further first.
4. **On the ground.** `+tv/scan`, `+tv/sound` or `+tv/drive` from the bay imply the launch (300 IC s,
   twelve real seconds, added to the preview); `+tv/launch` is for a pilot who wants to stand on the
   ramp first. The site map opens around the TV: terrain glyphs with footing (regolith, rock, ice) and
   craters, the hull as `H`, the TV as `@`, no ore anywhere yet; `+tv` reads "rig 1: block C (open,
   unclaimed), field C-NW, footing regolith — no readings here yet: +tv/scan reads the nine fields
   around you."
   **As built (SM.4a):** `+tv/scan` and `+tv/sound` typed with the rig in the bay are one act that
   prepends the launch (§4.3); `+tv/drive` and the site map are SM.4b's. A rig on the site's ground
   gains its own line under its rig line in `+tv`: "Block K (open), field K-NW, footing regolith —
   no readings here yet: +tv/scan reads the nine fields around you." The tenure word is the site's
   authored one (commons, open, or held by {org name}; claims are SM.6's and SM.10's), and the
   reading is the pocket's text if sounded, else the field's row if scanned. Two further states this
   document does not name are the lead's owner-critical default (SM.4a's D25), standing until the
   owner rules: "…; the ground here is still being mapped." (a cold site, which the surface tick
   then warms) and "…; the instruments cannot read the ground here." (a dark layer, no body table,
   or a site whose ground failed to build this boot).
   **As built (SM.10):** the tenure word reads the block's `tacsim.claims` record — "held by {org}"
   for a live org seed, "your claim" for the viewer's own, "claimed by {name}" for another
   character's — and falls back to the authored word for a block with no record, or a claim lapsed
   or past its term (R20: a lapsing claim reads open).
5. **Scan** (600 IC s, 25 real seconds). `+tv/scan` reads the nine fields (320 m) around the TV's own
   — 48 cells on each axis — that the mast can see: which family and how rich, as barren, lean, fair
   or rich with the count of ore pockets the word stands for, each field by name (`C-NW`). Footing
   sets sharpness: from rock a single word, from regolith a two-word band ("fair–rich") that never
   excludes the truth, from ice "no return". Decision: sound here, or drive to rock and scan sharper.
   **As built (SM.4a):** a field read as a band prints the union of the band's pocket counts, "C-NW
   fair–rich metals (5–16 pockets)", so the count never excludes the truth (§5; the lead confirmed
   it).
6. **Sound** (1,200 IC s, 50 real seconds). `+tv/sound` reads every pocket (40 m) within sixteen cells
   on each axis — a whole field from its middle — as barren, lean, fair or rich per family, drawn as
   `.` `1` `2` `3` (the digit is the pocket's class: tonnes a cycle until the pocket is drawn), each
   with a site address (`C117`). Everyone on the site hears a sounding as a bearing and a range band;
   `+tv/sound quiet` reads eight cells and is heard by nobody. Decision: which pocket, and whether
   rivals may learn where you think the ground is good.
   **As built (SM.4a):** the heard line reaches only the parties on the site connected at the time
   and is never queued, at most once an IC hour per sounder on that site (§5; the lead's defaults,
   standing until the owner rules).
7. **Drive** (five IC s a cell on regolith: 500 m in ten real seconds). `+tv/drive C117` previews the
   path, the time, the charge it costs and the charge left after the trip home; `+tv/drive C117 go`
   commits a frozen leg (a block, a cell or a bearing and a distance are targets too, for exploring).
   The glyph moves along it; "Arrived at C117."
   **As built (SM.4b):** `+tv/<n> drive <target> [go]` takes a pocket (`C117`), a field (`c-nw`), a
   block letter, a site cell (`140,96`) or a bearing and metres (`ne 300`); typed from the bay it
   implies the launch, as a scan does. A regolith kilometre is exactly 500 IC s and 2,250 cu. The
   arrival line reads "Rig 1 arrives at C117.", the operator's own reading of the pocket in brackets
   after the address when there is one (`ARRIVED`); `+tv/map` prints the drive view (§10).
8. **Mine** (1,800 IC s a cycle, 75 real seconds; rock takes twice as long). Open ground is yours to
   work under your licence until someone stakes it. `+tv/mine` previews the run from your own reading
   of the pocket — "C117 fair metal ore: 4 cycles, 2/2/2/1 = 7 t, 5 min; hopper 0 → 7 of 12 t; hold
   room after: 67 t; charge 71 → 63 % (55 % after the trip home)" — and `+tv/mine go` commits it: with
   no number, this pocket out, the hopper full or the hold's room used up, whichever comes first; a
   named count that does not fit refuses and names the count that does; on a pocket of two families
   `+tv/mine ice` chooses (the preview names the default). On a pocket you have no reading of it
   prints "No reading here: the first cycle will tell." and commits one cycle, whose core is the
   reading. While a run drills the pilot may scan or sound from the new stand, preview the next
   drive, read the registry, or work the second rig. Decision every pocket: work it out or move on.
   **As built (SM.7):** `+tv[/<n>]/mine [family] [cycles] [go]` (alias `drill`, as a switch or a
   first word, the family and the count in either order) and `+tv[/<n>]/mine stop`, which commits
   with no `go` (R7, an owner-critical default the lead accepted, standing until the owner rules).
   A rig in the bay is refused (`MINE_IN_BAY`): mining implies no launch. The cycle line above
   ("Rig 1, cycle 2 of 4: …") and this beat's blind sentence are pinned byte for byte; every other
   mining text is the planner's wording for the owner to see, `RUN_DARK` among them (§8). During a
   run only a scan or a sounding may start on the drilling rig, and a drive previews but refuses
   `go` (§6).
9. **Hopper full, go home** (a minute). `+tv/recover` previews the trip to the ramp, the entry into the
   bay and the unload as one command ("home in 25 s; then 12 t metal ore into the hold, 62 t of
   room") and `+tv/recover go` files them; the hold always has room for what a hopper carries, because
   no cycle starts without it. The charge recovers in the bay. Decision: another trip, or enough for
   today.
   **As built (SM.4b):** the trip to the ramp and the climb into the bay are one `Recover` act from
   anywhere on the site's ground, its preview naming the drive to the ramp and the climb into the
   hull's bay (`RECOVER_DRIVE_PREVIEW`, §4.3); the unload half is SM.8's.
   **As built (SM.8):** the recover's preview ends with the unload it files, `RECOVER_THEN` — "…;
   then it unloads 12 t of metal ore, run-of-mine into the hold, 74 t of room after" for a Dray-P's
   86 t hold with nothing else aboard (the label goods v3's) — or `RECOVER_THEN_PART` and
   `RECOVER_THEN_NONE` when the hold's room is short, and nothing for an empty hopper. The recover
   lands and tells `RECOVERED_UNLOADING`, the queued unload starts on the next fire and tells
   `UNLOADED` 600 IC s later; `+tv/1 unload [<t>] [go]` does it by hand (§7.1). An honest fill
   always fits; a hold staff have shrunk unloads what fits and says what stays (§6).
10. **Stake it, if the ground is good** (one real minute). From the TV standing in an open block,
    `+claim/stake` previews — "block C, your readings: C-NW sounded, C117 cored; soundings heard on
    this block since it last opened, before yours: 2; 2,000 cr now and 2,000 a term" — and
    `+claim/stake go` files it on the strength of your own hand-made readings. The block turns `#` on
    everyone's site map and only your rigs may mine it; rivals may still drive and sound. Renew it with
    `+claim/renew` (2,000 cr) once 50 t are worked in the term, or let it lapse and it reopens to
    everyone but you for a term.
    **As built (SM.10):** `+claim/stake` is one act over the whole block from a rig standing in it,
    with an optional declared family (the lead's ruling 1; OC7, OC8); its preview's fee reads
    "2000.00cr now and 2000.00cr a term (4000.00cr today)" (lane v12 data, OC1), its heard count
    reads "at least N" past a scan of 512 chart rows (OC12), and it prints the cooling line when one
    binds. The fee is a Public contract escrowed by the pilot and accepted by the site clerk for the
    Registry once the act lands (§9.3). The block's perimeter turns `#` on every site map (§10).
    `+claim/renew` costs 2,000 cr against the worked floor (§9.2); `+claim/release` lets a claim go
    at once, a lapse by choice with the same cooling (OC6); and the cooling binds the holder, anyone
    who acts for it and every character of the holder's gateway account (OC15). OC1, OC6, OC7, OC8,
    OC12 and OC15 are the owner-critical defaults the lead accepted, standing until the owner rules.
11. **Leave it running** (one real minute). `+tv/plan C117,C118,D family=metal_ore` files a fill plan:
    work these pockets in order (a block letter sounds its four fields and mines every pocket of the
    family that reads fair or better, nearest first), recover and unload at every full hopper, park
    when no cycle fits the hold's room or the list is worked out, within 288 IC hours; the captain's
    queue gains `await tv all`, so the hull can lift, fly home and sell while you sleep. One hold a
    plan, no more, and a plan is filed by hand.
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
IC-crossing rule are two L cuts (the arena map's gaps; doc 23 takes the first — the non-match
sortie lifecycle is the seal epoch of D-23-1 on the IC-clock hosts, this tick first, cuts S.1–S.4 —
and leaves the IC-crossing rule with §17). Not an atlas window: windows are eight
world-wide and clockless. Not a `DurableSchedule` per excursion for player trips: the schedule driver
polls at 250 ms, defers behind legacy persistence and discovers one candidate per poll
(`mush-server/src/world_task.rs:287, 3471-3487`), which is the production source's budget, not a
player's. The port tick (5 s, sixteen agents a fire shared with the clerks and the queues) keeps the
Registry's, the site clerk's and the queue's acts. On the IC clock a 500 m drive is ten real seconds
and a mining cycle 75; nothing is RAM-only, a restart resumes at the next fire with no rewind, the
site stays visible to the crew, and logging off is safe.

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
`deny_unknown_fields`, `def.rs:86-110`). Each site has its own lane `location` (role `extraction`, `body`
the body, `lat_mdeg`/`lon_mdeg` on it, south negative) and routes both ways to its port; a route to a
same-body site is a hop (§3.3), a route to another body an ordinary passage.

```
"ahuna_flank": { "kind": "location", "def": { "label": "Ahuna Flank", "role": "extraction",
                 "body": "ceres", "lat_mdeg": -10500, "lon_mdeg": 316200 } },
"ahuna_port":  { "kind": "port", "def": {
  "location": "ahuna_flank", "venue": "ahuna", "area": "neighborhood.ahuna_flank",
  "gallery": "neighborhood.ahuna_flank.room.pad", "routes": { "ceres": ["ahuna_ceres"] },
  "site": { "name": "Ahuna Flank", "issuer": "freeport_registry", "zone": "core",
            "tenure": { "default": "open" },
            "blocks": { "A": { "tenure": "commons", "worked_permille": 400 } } } } }
```

**As built (SM.2):** the role word is `extraction`, not `field`, in the prose and the example ("field"
is the geology's R1 level; the lead's override 1). The `site` block carries a required `name` equal to
the location's label and no `geology` key: the table lives on the body (the ground chart, below). The
validator requires a site's body to author `surface_mg`, `radius_km` and `geology`, and a site's issuer
and holders to be orgs of the pack. The example's block A exception shows the shape; Ahuna Flank's
blocks are all open as built.

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
only, never stakeable), `open` (workable under a licence until someone stakes it, then the holder's)
or `held` (an org's, naming it) — so one site may hold a commons corner, open ground and an org's
worked blocks at once. A block row may also carry `prospectivity` (a class shift applied to its
fields before the pocket pass, so a rim can be authored rich) and `worked_permille` (that share of
every pocket drawn into the site's spoil at opening, receipted, so a commons can open worked over with
its pits showing, §3.4).

**As built (SM.2, SM.3b):** the applier's part (g) lays the sixteen block place Things ("{site} block
{L}") DARK in the lane place's host room, **owned by the applying actor**, each recorded in the port
map as it is made; SM.6 makes `tacsim.sites` (`SITES_PRINCIPAL`, declared in SM.2) their industry
owner. The site clerk Thing is laid up beside them, and SM.3b adds a DARK "{site} spoil" Thing and its
vault (§4.3). Nothing is submitted; an update re-lays nothing recorded, and a recorded Thing gone
refuses at preflight. Block letters run A–P from the north-west.

**As built (SM.6):** SM.6 did not make `tacsim.sites` the block places' owner: the block place
Things stay owned by the applying actor, and `tacsim.sites` owns the deposits' endpoints instead —
the sixteen **block vaults** ("{site} block {L} ground", each backed by `Container(block place)`, no
scope, Active, laid by the applier's part (g) and recorded in `SiteRow.vaults`) and the spoil vault
(SM.6's D13; the lead confirmed it). A recorded block vault gone or not `tacsim.sites`'s refuses at
preflight; a store laid up before SM.6 gains its block vaults on the next apply (an owner-critical
default the lead accepted, standing until the owner rules).

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

**As built (SM.2):** six vendor-less site ports `<venue>_port`, each under issuer `freeport_registry`
(a Ceres org, 50,000 cr, faction freeport) and zone `core`, at coordinates as authored (the lead's
default, standing until the owner rules): on Ceres, Freeport Commons 20.3 N 240.4 E, Kerwan Floor
10.8 S 123.9 E, Ahuna Flank 10.5 S 316.2 E, Juling Shadow 35.9 S 168.6 E and Occator Rim **25.4 N**
239.3 E, with the Freeport itself at 19.7 N 240.4 E (role `port`); on Vesta, Rheasilvia Commons
75.0 S 301.0 E. Occator Rim was first authored at 21.3 N, which put its pad on the crater's floor
12 km from the centre; the lead's fix round moved it to 25.4 N, on the northern rim about 46 km
north of the crater's centre, before lane v9 latched anywhere. The Commons is sixteen commons blocks
at `worked_permille` 400 (acted on in SM.6), Rheasilvia is all commons, and the other sites are open. At Occator Rim the inner blocks F, G, J and
K are held by `occator`, with no distinction yet between the two its drones work and its reserve, and
the outer twelve carry +1 `prospectivity`, the only authored shift (the lead's default, standing until
the owner rules): "metals and carbon" at Kerwan and Ahuna and "ice-rich" at Juling are whatever the
key and Ceres's table make them.

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

**As built (SM.2):** `BodyDef.geology` is an inline table — SM.1's wire, checked at load — not a
table name, and `radius_km` is required with it. Surface figures are authored on eight bodies: Ceres
28 mg / 470 km, Vesta 25 / 263, Pallas 21 / 256, Luna 165 / 1,737, Mars 379 / 3,390, Titan 138 /
2,575, and Phobos and Deimos ceiled to 1 mg (the lead's default, standing until the owner rules).
`+chart/ground` and the `body.ground` feed are SM.9's.

**As built (SM.9):** the ground chart landed. `+chart/ground [<body>]` lists the body's sites, each
with its survey words, its tenure, its passage and the viewer's best reading, and `+chart/ground map
[<body>] [<family>]` draws the grid: the wash of **one family** a print (metals by default) as `.`
barren, `-` lean, `+` fair and `*` rich, the ports by the initial of their label's last word and the
sites by their first free initial in entry-id order, a port keeping a shared cell and the hidden
markers listed beside it ("Also at F: C Freeport Commons, O Occator Rim."). `body.ground` is a pulled
grid feed at `cell_columns 1` (56 × 28, one province a cell, params `{ body, family }`), registered
after `site.map` and `tv.status`; line mode prints the body map at one column a cell. The frame
stores its rows **south-first**, like every grid frame (`sky.chart` included), and every renderer
prints north up: the planner's north-first rule (its D17) was reversed at the judge, the lead
accepting it. The passage is planned at the viewer's hull's load on every surface, `+chart/ground`
and `body.ground` reading one `vantage_reading`, and planned empty, with the port's programme hull, for a walker without a charter; the site the hull stands
at carries no passage and reads ", where you are."; and the best reading is computed from the keyed
site context and the viewer's own row, never a warm site, so it prints after any restart, each site
row carrying it whole as `best_line`. From a site's pad every other site reads "no passage from
<site>" and the port line never shows the passage home, since site ports route only to the
Freeport: a routing and text question the owner holds (§15). The glyphs, the marker letters, the
shared-cell listing, the body map at one column a cell and every `GROUND_*` text are the lead's
owner-critical defaults, standing until the owner rules.

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

**As built (SM.1):** one HMAC-SHA256 per (level, cell, family) over `"rustmush.geology.v1\0" ‖ body ‖
0x00 ‖ level u8 ‖ x i64 LE ‖ y i64 LE ‖ family u8`, read as four little-endian `u64`s; level 0 is the
province, 1 the field, 2 the pocket and 3 the reading band's side (§5), and every coordinate is
normalised by its own level's width before it enters the HMAC (x wraps, y never). The fingerprint is
SHA-256 of the 32 key bytes, pinned with the key version in the tacsim module document
`tacsim:geology` at the first boot that has a key (a boot-time module document, never a submission)
and compared at every later boot. Five disabled reasons — no `[tacsim.geology].key_dir`, a key
directory that did not load, `tacsim:geology` unreadable, a mismatch, the layer off — read on the
readiness report's `geology:` line, reported and never gated; `[tacsim.geology].key_dir` has **no
default**, and the key is loaded only when `[tacsim].enabled` (§15 Q-20-12). Staff read truth through
`@geology <body> <x>,<y> [province|field|pocket]` over the body-fixed pocket lattice (no site shift
applies); the site-address form waits for SM.4, which binds cells to pockets.

Generation is **top-down, so readings nest**, and one word set — **barren, lean, fair, rich** —
names richness on every surface (the code's P0–P3 and A–D never reach a player): a *province* (one
cell of the body's 56-by-28 chart, 6.43°, 1,568 a body; the R0 level) carries per-family
prospectivity from the body's `geology` table (Ceres: volatiles fair to rich nearly everywhere,
metals lean to fair in provinces, carbon feedstock lean, strategic elements barren with rare lean;
Vesta: metals fair to rich, strategic lean to fair, volatiles lean, carbon barren); a province
conditions its *fields* (320 m, the R1 level: per family the count of ore pockets the field holds —
barren none, lean 1–4, fair 5–8, rich 9–16 — shifted by the block's authored `prospectivity` if any); a
field conditions its *pockets* (40 m, the R2 level: barren, lean, fair or rich, with tonnes lean 3–6,
fair 6–12, rich 12–24; a pocket may carry two families). So a block of four fields holds **at most 64
ore pockets per family** by construction, and at most 64 × 24 = 1,536 t; a whole site some thousands
of tonnes per family, most of them lean. Doc 10 4A's four raw families — metals, volatiles, carbon
feedstock, strategic elements — are all mined (the goods rows in §7.3); "what you get depends on where
you are" is the body, the family, the field and the pocket, not a digit.

**As built (SM.1):** the body-level pocket lattice this document left undefined is columns =
⌊⌊radius_km × 1000 × 710 / 113⌋ / 320⌋ × 8 and rows = ⌊columns / 32⌋ × 16 — whole fields around the
equator and on each side of it: Ceres (470 km) 73,824 × 36,912, Vesta (263 km) 41,304 × 20,640. A
pocket's province is (x × 56 / columns, (y + rows/2) × 28 / rows); SM.2 maps a site's
`lat_mdeg`/`lon_mdeg` onto the lattice and snaps its origin to a field corner. The authored
`prospectivity` shift is one `i8` per block in −3..=3, applied to the field word before the count is
drawn and saturating at barren and rich, and it is **never applied to a family whose province word is
barren**: a rich-authored Ceres block never grows strategic ore, while a negative shift still lowers
any family. That is the lead's technical ruling (SM.1's decision 13, read from §3.1's "the strategic
shelf is Vesta's" and this section's Vesta and Ceres sentences, where this document does not say),
standing until the owner rules; the table validator refuses a `field.barren` row that weights any ore
word, so the rule holds for any lane table, and SM.2's tables author `field.barren = {barren: 1000}`.
A pocket carries at most two families, placed by keyed rank in family order; the count and tonnes
ranges are code constants, not table data.

The site's terrain composite (background plus features) is built **off-loop** with the atlas's pure
`capture()` and `composite()` (`mush-atlas/src/window.rs:201, 521-688`) from the region pack and
held in a RAM LRU of sixteen (about 384 KiB a site); the site's geology table (4,096 pockets by four
families, two bytes each) is built beside it once per key version. Both are derived, never stored,
re-derived at boot. One finding on the way: the composite casts background elevation with
`i16::try_from(..).unwrap_or(i16::MAX)` (`window.rs:546`; `:470` is the harmless tile-mean twin),
which turns a low into a cliff, and background elevation is `datum_cm + noise` with `datum_cm` an
unbounded `i32` (`background.rs:36`, `record.rs:306`); the recipe validator requires `datum_cm ±
relief_cm` inside `i16` and the cast saturates symmetrically (SM.1).

**As built (SM.1, SM.2):** the cache is keyed by (region, revision, key version), the revision
digesting the whole captured `CompositeInput` and the site spec, and a replaced body table drops that
body's cached sites (`install_body_geology`). SM.1 landed the in-RAM LRU of sixteen, the pure builds
(`build_site_ground`, `site_geology`) and a synchronous door; the off-loop driver is SM.4's, its first
consumer (§13). `RegionRec::validate` refuses `datum_cm ± relief_cm` outside −32,768..=32,767 cm — at
the applier **and at the atlas boot load**, where a failing stored region is dropped loudly rather
than clamped — and both elevation casts saturate symmetrically through one helper. SM.2's
`build_site_ground` refuses a region that is not a surface of the spec's body, caching nothing.

**As built (SM.5):** the body geology tables install at boot and wherever the definition applier
creates or updates a `body` entry (`surface::body_geology_of`, the one builder `lane_geologies` and
the applier share; RAM only, skip-equal, a failure warned), whatever else the lane version changes;
the port applier's part (g) no longer installs them (SM.2's residual).

**As built (SM.4a):** no act's path builds a site ground. The command, the start's offer and the
`+tv` status read a non-building peek (`site_ground_cached`); a cold site answers that its ground is
still being mapped and is wanted, and the surface tick warms at most one wanted site a fire (wanted
at landing, at boot for every site with a programme hull docked, and on a cold peek). The judge
measured the warm peek at about 12 ms in debug and 408 µs in release, a full serialise-and-hash of
the composite on every `+tv`, scan command and offer projection; the lead's fix round made it a
capture plus one equality check against the sixteen-entry cache, which now keeps each site's
captured input beside its ground (57–71 µs in debug, 19 µs in release; a cold build about 85 ms in
debug and 2–3 ms in release, and a cold peek still hashes once), so no per-region revision cache was
needed. A build failure darkens the site for the boot with one warning (D25), and since the round a
capture failure does too. A
true off-loop keyed runner is SM.12's cost-lane call on these figures. SM.4a's `surface::address`
binds cells to pockets, fields and blocks (the ramp cell is pocket `K1` in field `K-NW`); `@geology
<site> <addr>` moves to SM.4b with the typed address parse.

**As built (SM.12c-lane):** the true off-loop runner, the cost lane's decision (a), was not needed
at this cohort. The 30-TV month's site warm max was 4.5 ms (4,530 µs, the builder's run) and its
home warm max 8.9 ms, and C4's release capture plus build max 4.0 ms (p50 3,226 µs, p99 4,031 µs),
all against the 20 ms line, so the site ground stays built on the surface tick, one wanted site a
fire, measured. SM.4a's per-region revision cache is the landed peek, recorded "landed as SM.4a's
pre-commit item 1 (capture + equality)": warm 22 / 67 / 67 µs and cold 405 / 841 / 841 µs in
release (C4), a p99 of 67 µs warm and 841 µs cold. The site-ground LRU now counts its builds and
evictions (`SiteCacheCounters`, RAM only; `forget_body` counts no eviction): under C4's eighteen
grounds it reads 54 builds, 38 evictions and 16 entries, and over the month 6 builds and 0
evictions (§16). §16's "builds stay off-loop" therefore reads "they stay on the tick, measured".

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

**As built (SM.2):** only a leg with a site end meets the law, and only a site end pays the ascent,
√(g·R) ceiled (Ceres 360 m/s, Vesta 254): Ceres Freeport is a port, not a site, so no pinned port leg
moved (the lead's ruling). At the rail Ceres reads 1,785 ‰ (Luna 303, Titan 362, Mars 131), and its
band edge laden is exactly 672 ‰ of throttle; Phobos is authored at 1 mg, not 0.6. The hop is 5,322
IC s and 1 t out at 86,580 kg and 6,893 s home for this paragraph's loads, 7,200 s at the rail.
**Landed life support is charged at lift**, inside the pilot's own departure burn
(`PassagePlan.landed_crew_days`): souls × whole IC days settled, capped so the locker keeps the leg
home; the raw locker, the gauges and the manifest stay full while settled, and every stores line
prints the effective figure. The once-per-stay notice is the arrival notice to an absent tenant, with
its stores line; nothing fires at the instant the cap binds. The stores print on the arrival line,
`+ship` and `+ship/plan`, and on `+tv` from SM.3a. Three rules are new: **a departure for a site
holds back the leg home** — planned at the least lifting throttle, the worst of the next 30 IC days
when it crosses bodies — refused with `SITE_TANK_SHORT` or `SITE_STORES_SHORT` and printed by
`+ship/plan` as `SITE_HOME`, and `+ship/plan` and a refused departure for a site print a
**reserve-aware throttle hint** (the highest throttle at or above the landing floor at which the
outbound burn plus the reserve fit, else the first later IC day in `+chart`'s 365-day window on which
the floor fits), while the queue's sailing-plan dry run reads the same site refusals the fire holds
with; **nobody steps out onto a pad** (`LEAVE_PAD`); and **a site whose pad is not ready refuses
departures** (`SITE_CLOSED`) and never gates `+coop/enrol` (the lead's override 3). The charge at
lift, the arrival notice as the once-per-stay notice, the leg-home reserve and `LEAVE_PAD` are the
lead's defaults (SM.2's decisions 4, 8, 7 and 10), standing until the owner rules; Rheasilvia's
reach under the reserve is §15 Q-20-10's note.

### 3.4 Depletion — D-20-5

The reserve is **one `OreDeposit` per (block, family)**, its commodity **the family's ore good**
(§7.3), its live reserve the inventory pool at the deposit's endpoint, **credited once when the site
opens** — never on the player's path and never by the applier, since doc 18 refuses submissions from
the applier or the boot pass (`18-trade-run-vertical-slice.md:3493`). The opener is a composed
**system opener** in the stock clerk's shape: `IndustryModule::with_system_opener(..)` admits
`DepositOpen` from a per-site **site clerk** Thing holding the opener grant, acting through
`Fire::as_principal` on the port tick's stock arm (`port_tick.rs:1047`; the stock mint's own pattern,
`port/stock.rs:333-438`), one deposit a fire with the site's server-authored rows as fixed parameters
and a receipt per `(site, block, family)` as the replay guard, so a re-apply opens nothing twice; the
readiness line reads a site as "opening, 41 of 64" until its deposits exist, and the site admits no
TV before. `tacsim.sites` is the **owner** principal of the block places, the deposits' place objects
and the spoil vaults, not a submitting actor (an authoritative submission's actor is an `ObjId`,
`mush-core/src/module.rs:4964-4975`). Each block deposit is opened with accessibility 100 and a
one-point curve `[{cumulative 0, per_cycle 1}]`, so `extraction_authorized`'s body test
(`licence.rs:167-172`) passes until the pool is empty and the pocket cursor alone sets the tonnes.
The reserve credited is the **gross** sum of the block's pocket tonnes for that family, computed
off-loop from the geology table and re-derived server-side, never player input; an authored
`worked_permille` is then a second receipted act of the clerk, a pool **transfer** of that share
from the deposit to the site's spoil vault (§4.3) with the drawn tonnes written to the pocket cursor,
so the invariant below holds from the first sweep and the matter is accounted for (the commons's
spoil heaps are real). Up to four deposits per block place and 64 per site sit far inside
`MAX_ORE_DEPOSITS_PER_PLACE` 8 and the 4,096-deposit walk.

**As built (SM.6):** there is **no opener grant**: the composed `SiteOpener` admits exactly the live
Thing recorded as `SiteRow.clerk` of a latched lane site, the clerk's identity its whole authority
(D13; the lead confirmed it). The clerk acts on the port tick's own `Arm::Site`, not the stock arm,
one act a fire through `Fire::as_principal`, the site clerks sharing `MAX_AGENTS_WORKED_PER_FIRE` 16
with the other arms; an offer carries one row — the next pending row in plan order, its receipt
`"{entry}/{letter}/{family}"` and the deposit id derived from it — never the whole site (D15).
`tacsim.sites` owns the sixteen block vaults and the spoil vault, the deposits' endpoints; the block
place Things, the deposits' place objects, stay the applying actor's (§3.1's note). A deposit opens
only per (block, family) whose pockets hold that family's ore: **up to 64** a site, 48 on five of
the six shipped sites and 64 at Kerwan. A deposit's existence at its public id is therefore a
barren-ness bit, which SM.7's `+tv/mine [family]` and SM.10's stake must not echo: each answers a
barren and an ore-bearing (block, family) with the same line and fee (D14, an owner-critical default
the lead accepted, standing until the owner rules; opening all 64 with barren deposits at a reserve
of 0 would need an industry record change). The gross reserve is read through the warm site ground
(reading rule v1's pocket evaluation), never built on the tick. The worked share is per pocket:
⌊t × ‰ / 1000⌋ of every ore pocket of the family, the summed share moved by one pool transfer
(`inventory.transfer`) from the block vault to the spoil in the same bundle as the cursor insert,
with one cursor row per pocket that gave at least a tonne (D6; no nearest-first). The readiness line reads "opening, {done} of {total}." until the last deposit exists, the pad closed
meanwhile; after a boot a keyed site reads "opening: its ground is still being mapped." until the
surface tick warms it, and a ground, plan or issuer that fails reads "opening: its ground could not
be mapped (see the log).", paged once. With the geology layer dark nothing can open, so the line
reads Ready ("no deposits while the geology layer is dark") and the pad stays open (D2, an
owner-critical default the lead accepted, standing until the owner rules). A keyed site opens one
deposit a port-tick fire, then works its worked share one transfer a fire: 49 fires for an unworked
site, 65 at Kerwan, and about 97 for the worked Commons (some eight real minutes at 5 s a fire),
once. Every deposit records its site's issuer as its jurisdiction (§9.6).

**As built (SM.12b):** the site clerk's arm gains two items for a site with a yard (§9.6's note):
after its held seeds it opens the house's standing licence, then the house's survey one block a
fire, and only then seeds the papers; the two items answer nothing whenever they cannot progress,
so the papers never wait on the house (D26, the lead's ruling).

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

**As built (SM.6):** the sidecar is the strict family `tacsim.pockets`, its key text industry's
deposit id as `{:016x}` and its pockets the block's 1..=256 address numbers (`PocketCursor`, at most
64 `CursorRow { pocket, drawn_t ≤ 24, lock }`). The trait is `ExternalDraws::drawn(&self, ctx:
&ModuleCtx<'_>, deposit: &OreDeposit) -> Option<u64>` (D11), the typed family readable only through
a module context; tacsim's `SurfaceDraws` answers 0 for an absent cursor and the cursor's total for
its own deposit's. A mismatch quarantines **every deposit sharing the place and the `external`
name**, with three causes: `DrawsMismatch`, `DrawsUnanswered` (an unreadable pool family or a `None`
answer) and `ExternalUnwired` (no seam composed). Industry's own `@extract/start` and
`@extract/cycle` refuse a body whose `external` is set, closing SM.5's residual F12 (ii). The sweep
reads `tacsim.pockets`, and the port tick and readiness read deposits, through two typed-state
delegations on the TacSim roster only — industry → tacsim (the port tick and readiness read
deposits) and tacsim → industry (the sweep reads `tacsim.pockets`) — each granting write as well and
held to reads by `tests/deposits.rs`'s D10 source tripwire, a tripwire and not a
proof (D1, an owner-critical default the lead accepted, standing until the owner rules). One bundle
may now open any number of new pools at one endpoint: mush-inventory's `transfer_many` stages the
endpoint's census once a bundle (SM.3b's round's F3, closed), and the dump runs on it too.

**As built (SM.7):** the lock's home is the cursor row, `PocketLock { tv, ordinal }`, written by the
start's bundle and released at the run's close, a row neither drawn nor locked removed; a start
reads the pocket's rows and every other rig's run record (R4, §6's note), not the row alone. The
preview never echoes the barren-ness bit above: a barren pocket answers byte for byte as an
ore-bearing one the operator has not read (the lead's ruling 2). The pits stay derived from the
cursor through the map's one `pit_at` rule; a pocket worked out in one family and untouched in the
other reads `V` on every map until SM.9's two-family rule.

**As built (SM.9):** the two-family rule stands as built, with no code changed: a pocket worked out
in one family reads `V` on every map whatever its other family holds, pinned by
`sm9_a_pit_worked_out_in_one_family_reads_v_whatever_else_it_holds` (SM.9's D9, the lead's
owner-critical default, standing until the owner rules).

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

**As built (SM.3a, SM.3b):** the TV is fielded into the catalog — it binds and mints a
`DefinitionRef` — but has no burn row, and a drive-less unit refuses a passage with `NO_DRIVE`. "The
operator's control lease" is read as the live lessee (`TvLease.pilot`) aboard the settled hull: no
mush-sim F3 lease is taken on the TV, and "aboard" gates launch, recover and dump alike (SM.3b's
decision 14, the lead's default standing until the owner rules; §8 and §19 item 3 read the same way).

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
after the act *and after the trip home*, and the act is refused when that would fall under 5 %; the
charge is **reserved in the act's own authoritative start** (§8), so two acts can never both spend the
same margin, and a TV cannot be stranded by its battery, by construction.

**As built (SM.3a, SM.3b):** tools are strict typed documents pinned by digest (`tacsim.tools.v1`),
not packs, and a third row, `battery_mk1`, carries the charge figures as data. "Read by the surface
engine now" was not yet true in SM.3a, whose tools only their tests read; SM.3b's acts read the
battery row (the one battery source), and the drill and scanner rows wait for SM.7 and SM.4. The
reserve rule is enforced only in `tacsim_surface_act`'s prepare; a spend of zero (cost plus home
equal to 0) always passes, so a recover or a dump is never refused by it, and a rig on the ground
comes home and dumps even with its battery row unreadable. `reserved_total` reads only the act in
flight until SM.7's cycle slot extends it.

**As built (SM.4a):** the scan and the sounding read `scanner_mk1` (600 IC s and 1,000 cu; 1,200
IC s and 2,000 cu; radii 48, 16 and 8), so only the drill row still waits, for SM.7. The reserve
rule's trip home is `home_cu` 0 until SM.4b's drive. A scan or a sounding typed with the rig in the
bay reads the bay's credit only up to `started_ic + LAUNCH_S` (§4.3). A scan or a sounding started
from the ground whose battery row stops resolving still settles, subtracting its reservation
(`settle_spent_out_of_bay`), where the judge found it retrying for ever; the lead's fix round did
it, and an implied launch still in the bay fails closed as SM.3b's launch does.

**As built (SM.4b):** the reserve rule's trip home is real, superseding the SM.4a note's `home_cu` 0:
the exact least cost from the act's end stand to beside the ramp, read from a per-(site terrain,
ramp cell) **home field** — a multi-source reverse Dijkstra from the beside-ramp cells over the
atlas's own move costs, built once on the surface tick and cached (32 fields at most), keyless from
the atlas terrain when the geology layer is dark (P14, an owner-critical default the lead accepted,
standing until the owner rules). A drive to a stand with no way
home is refused (`NO_WAY_HOME`); a scan or a sounding from a stand already occupied with no way home
reserves the conservative bound (1,226 cost a cell on the shipped table). The 5 % floor is never
applied to a recover: it is refused only when its out-of-bay charge cannot cover its own leg
(`RECOVER_SHORT`), and it never reads the battery, so a recover with no readable battery row spends
nothing for its leg (P6, an owner-critical default the lead accepted, standing until the owner
rules; and the builder's decision 3, a default the lead accepted and the owner should see beside
P6). Driving reads the unit's tracked `max_speed_cms` 300 and the
battery row's 15 cu per nominal cell, no constant of its own (§11).

**As built (SM.7):** the drill row is read: `drill_core_mk1`'s `cycle_s` and `rock_factor` set a
cycle's pace (`mine::cycle_secs`: 1,800 IC s, 3,600 on rock), its yield row the tonnes, and the
blind room is derived from it (§6). `reserved_total` is the act's reservation plus the run's
remaining one, saturating (§8), and a run reserves its charge and its trip home at its start.

**As built (SM.8):** an unload is `UNLOAD_S` 600 IC s, a code constant (no content digest moves),
spent in the bay: it reserves nothing and reads no battery, and the bay regains 600 × 50,000 / 3,600
= 8,333 cu over it, capped at the capacity (pinned by U11), where §4.1's Draft paragraph rounds it to
8,300.

### 4.2 The hopper vault

A single-room store interior anchored on the TV and a vault on it, laid up by the hull recipe
(`declare_space`, `port/hull.rs:685-714`; `create_vault` as `+coop/enrol` does,
`port/coop.rs:1520-1553`), **owned by the system principal `tacsim.vehicles`** — never the pilot and
never the hull's titled owner (a graduate's hull is titled to the player, `hull.rs:797-801`). No
player passes `may_take` on it, so `live_endpoints` never offers it, `@inv/move` cannot teleport ore
between hopper and hold, and doc 18's one-vault premise (D-18-37) survives without building P-H.
Only two server-authenticated acts write it: the cycle's transfer in (§6) and the unload out (§7).
Every quantity in it is a pool, so gauges, capacity and payload read it at once.

**As built (SM.3a):** the hopper is laid up by the applier's part (h) in Occator's unanchored shed,
not by the hull recipe nor by `create_vault` as `+coop/enrol` does: its vault id comes from the
applier's allocator, owned by `System(tacsim.vehicles)` with scope None. Eight rigs, `OWP-TV-01..08`,
stand in "Occator Vehicle Shed", each a Thing, a unit, a cab and a hopper space, titled to Occator. SM.3b's `+tv/dump` is a third act that writes it, out to the site's spoil (§4.3).

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
free (`mush-sim/src/journey.rs:3-9`); its **slot number** (1 or 2) is its name on every verb and line.
`+coop/refit prospector|standard` at the desk swaps an enrolled charter between the Dray and the
Dray-P as lane data (refused under way or with cargo aboard). A TV comes from **Occator's lease** —
doc 11 §7's units table leases Occator's extractor drones to claim-holders, and a terrain-vehicle
lease extends it (Q-20-6): `+tv/lease` at Occator's counter is the player's own act under the
server-authored lessor, the `+coop/enrol` shape (`port/coop.rs:1-27`), 1,400 cr a 168-IC-day term
(re-debited at the next dock, else the deposit is forfeit) and a 5,000 cr deposit held, refused
without an Active licence or a free bay slot, laying the TV into the bay; `+tv/lease return` refunds
the deposit with the TV in the bay and the hopper empty, and is refused elsewhere. Two leases are two
TVs; the second is the lever that makes a live hour a juggling act.

**As built (SM.3a, SM.3b):** `+coop/refit` re-binds the chartered hull's tactics unit in place
(`mush_tactics::module::refit_unit`) rather than swapping it "as lane data"; the lane carries a
`refits` row on the programme. The bay room, its space and its `rigs` compartment are laid on
**every** programme hull at apply, standard Drays included — hidden, empty and exitless — and the
egress is definition data (`EgressDef`); no `EgressSpec` is written. The refit also refuses with
provisions in the hold, with a rig in the bay (or a live lease record) and away from the desk.
Occator's counter is the Ceres Exchange Hall, its rigs `OWP-TV-01..08` in "Occator Vehicle Shed".
The lease is an economy lease with a 1-minor Manual rent and one public post carrying the term fee and
an engine-owned `tacsim.tv.deposit` hold (the first game namespace in mush-economy's frozen
`ENGINE_OWNED_RESERVATIONS`, §18); the term is re-debited **at the pilot's next `+tv/lease` act**, not
at the next dock (the at-dock hook is SM.3a's residual R1); a forfeit captures the whole deposit and
the rig waits to be returned; the return is keyed on the lessee's own record, so a Revoked pilot can
still hand a rig back. Until SM.5 the licence refusal is a seam that always answers Ok
(`licence_seam`). The bay on every hull, the Exchange Hall counter and the rigs' names, the deposit's
namespace and the re-debit at the next lease act are the lead's defaults (SM.3a's D1, D11/D17, D18
and D8), standing until the owner rules. The slot switches are `/1` to `/4` (`MAX_BAY_ITEMS`), not
"1 or 2": a slot past the hull's bay answers `NO_RIG`, and a bare act addresses the rig last named,
else the only rig; the pointer is RAM, so after a restart a two-rig act with no number refuses
(SM.3b).

**As built (post-buyout mining):** refitting, taking a rig lease and empty-bay
status also accept the current active title to a programme hull. A graduation
record is not authority: a former owner cannot use it after title passes on.
The services read the current hull and its stowage-backed, active, pilot-owned
hold, rather than the enrolment's historical hold. Fleet assignments do not
grant these services. Normal berth, flight, cargo, licence and lease checks
still apply; returning a rig remains keyed on the lessee's own live record,
including after loss of helm.

**As built (SM.5):** the seam is filled: `licence_seam` became `licence_check`, and `+tv/lease`
(preview and `go`) is the one act besides mining, staking and renewal that reads the licence, at
check 6 of `plan_take`, before the slot and before the term settle — refused `LEASE_LICENCE`, or
`LICENCE_BOOKS_DARK` when the licence cannot be read; launch, recover and dump read none. A lapsed
licence therefore refuses the lease before SM.3a's R1 term settle, so a lapsed pilot's past-term rig
is neither re-debited nor forfeited until they relicense or R1's at-dock hook lands (SM.5's decision
21, an owner-critical default the lead took, standing until the owner rules).
**As built (RK-1):** the licence no longer blocks the settle. Check 6's refusal carries the term
settle `plan_settle` makes, and `+tv/lease` runs it (`go`) or shows it (preview) before the
licence's line, so a lapsed pilot's past-term rig is re-debited or forfeited at their next lease
act like anyone's; with a settle due the licence's line closes the reply rather than refusing it
(dispatch keeps no success line of a handler that errs), and with none due it is the plain refusal.
The at-dock hook (SM.3a's R1) is still not built: a pilot who never types `+tv/lease` again is not
settled, licensed or not (doc 20b DF-2).

**As built (SM.3a, SM.3b): a charter's end with rigs aboard.** `+coop/buyout` refuses with
`BUYOUT_BAY` and `+coop/release` with `RECALL_BAY`, both failing closed when the hull's bay cannot be
read; `@coop/revoke`, the dormancy recall and the return arm repossess every rig first. Each deposit
settles before its lease ends: released within the paid term, captured whole past it, and the engine
never debits a fee. A non-empty hopper, an unreadable or unlisted record, a deposit that will not
settle, or a rig out or mid-act holds the hull and pages staff with `BAY_PAGE`; the rig is never
stripped. The deposit rule (SM.3a's D16) and the hold-and-page (SM.3b's decision 13) are the lead's
defaults, standing until the owner rules.

`+tv/launch` is an egress to `Direct` on the site's passive domain at the ramp pose
(`mush-sim/src/commands.rs:2060-2159`), legal only while the hull is settled at a site and the
operator is aboard, and implied by the first `scan`, `sound` or `drive` from the bay; `+tv/recover`
(§7.1) places it back. Two typed **departure refusals** keep the frozen ticket honest: `TV_OUT` (a bay
slot's TV is on the ground) and `HOPPER_LOADED` (a hopper holds ore: unload or dump first) — backstops
that the hold-room gate (§6) keeps from firing on an honest fill. `+tv/dump` moves the hopper's pools
to the site's **spoil** pool, a system vault per site — conserved, unrecoverable, a decision with a
preview. An **excursion** runs from launch to park and is identified by `ExcursionId (tv,
launch_ic)`: the sortie identity doc 17 needs (D-UM-45, a hangar to a hangar), the bay being the
hangar. Doc 23 (D-23-1, D-23-3, D-23-7) makes this excursion the first IC sortie: the epoch opens
when the rig leaves the bay at the launch's end, not at the excursion's start, closes
`ReturnedToBase` at the recover (or `RecoveredToBase` by staff `Home`), and its seal line names the
carrier's open sortie, since a settled Dray-P is in the field, not a hangar.

**As built (SM.3b):** the launch is the surface tick's `mush_sim::place` at `ramp_pose` — the pad
centre (128,000, 128,000 cm, facing north) plus the bay's `EgressDef` offset, so the Dray-P's stern
ramp is at (128,000, 124,500, 180,000) — not mush-sim's `egress_parent`, which folds the egress pose
into a posture at the origin; it takes 300 IC s (`LAUNCH_S`). The implied launch is SM.4a's, which
prepends `launch_plan`. `+tv/recover` is the placement half only: from the ramp's own cell or the next
(Chebyshev), in 300 IC s (`RECOVER_S`); a rig further off is told about how far it is, and the drive
home is SM.4b's. A third departure text, `TV_BUSY` (a rig mid-act), sits beside `TV_OUT` under the same
`TvOut` kind, and `TV_AWAY` names staff as the remedy where `+tv/n recover` cannot work (a rig still
logged out while the hull is away from a site's pad); all of them, and `HOPPER_LOADED`, refuse
`+ship/plan`, `+ship/undock`, `+ship/lift` and the queued depart, and `HOPPER_LOADED`'s remedy names
only the dump until SM.8 adds the unload. Each site's spoil is a DARK "{site} spoil" Thing in its host
room backing a vault owned by `System(tacsim.sites)` with no scope (`CustodyRef::Container`), both
laid by the applier's part (g) and named on the Site readiness line, so a missing spoil closes the pad;
a dump takes no IC time (`DUMP_S` 0), moves each pool's `available` quantity, and is recorded refused
(`SpoilGone`), nothing moved, when the spoil will not take the load. An excursion runs from a launch's
start to a recover's `Done` (parking is SM.11's): a refused launch (`NeverLeft`) closes it, a recover
from beside the ramp needs no open excursion, and a recover of a rig in the bay with one still open
closes it. `RECOVER_S` 300, `DUMP_S` 0, a past-term Active lease that still launches (recover and dump
are never lease-gated) and the launch's place at `ramp_pose` are the lead's defaults (SM.3b's decisions 9, 10,
11 and 25), standing until the owner rules.

**As built (SM.4a):** the implied launch landed. A scan or a sounding typed with the rig in the bay
is one act whose plan prepends `launch_plan` (its refusal order unchanged), `LAUNCH_S` longer,
opening the excursion at the start exactly as a launch. The bay credits charge only up to
`started_ic + LAUNCH_S` (`charge_for_act`, `spent_after_launch`); the reserve and the preview's
charge read that same instant, and `+tv` shows no later bay credit while the act is in flight
(`charge_shown`). An implied launch whose rig never left the bay is refused `NeverLeft` with its
excursion closed; one moved off its stand after leaving the bay settles its charge out of the bay at
the launch half with nothing spent and keeps its excursion open (§8).

**As built (SM.4b):** the drive home landed. `+tv/recover` from anywhere on the site's ground drives
to beside the ramp and climbs aboard as **one** `Recover` act carrying a frozen leg (`secs` the leg's
plus 300), placed back in the bay at the due instant, even while the geology layer is dark (its way
home keyless, §4.1); `RECOVER_FAR` remains only for a cell with no way home at all, and
`RECOVER_DARK` answers when the way home's terrain cannot be mapped this boot. A drive typed from
the bay implies the launch as a scan does.

**As built (SM.8):** the unload landed (§7.1), and `HOPPER_LOADED` names it first for a hull on a
site's pad ("…: +tv/{n} unload moves it into the hold, or +tv/{n} dump empties it onto the
spoil."), while `HOPPER_LOADED_AWAY` ("…: staff must empty it.") answers anywhere else, both under
`DepartRefusalKind::HopperLoaded`: an unload works only on a site's pad, so at a port neither remedy
works (R5, an owner-critical default the lead accepted, standing until the owner rules; if the owner
allows unloading at a dock, the gate and the port line go away in one change). A queued unload is
not busy to the departure gate.

---

## 5. Knowing — D-20-8

A player never sees the ground; they see **their own readings**. The store holds *coverage and
provenance*, not readings: one chart document per `(principal, body)`, `tacsim:chart:<principal>:<body>`,
with at most 1,024 field rows `{field, fidelity, first_local_at, last_at, provenance ≤ 4}` and at most
64 pocket blocks of 16 by 16 one-byte cells (tier and pit seen), some 70 KiB worst case; at the caps
the server refuses and `+tv/forget <site>|<body>` frees room. Readings are **re-derived at render**
from the cached geology under the row's pinned reading-rule version, so a reading never drifts from
the ground and the store never holds truth. The `+tv` status line always says where each rig stands —
block and its tenure, field, footing — so a scan's rows and a stake's block are never a guess.

**As built (SM.4a):** the chart is the typed, authoritative family `tacsim.chart`, one `ChartRecord`
per **(character, site)** keyed `{principal}/{site}`, not a document per (principal, body): a
per-body row's worst case in canonical JSON runs past the family's 256 KiB cap (the lead's ruling
3). A site's row holds at most 64 field rows and 16 pocket blocks by construction and encodes under
its 64 KiB record cap; this document's per-body caps (1,024 field rows, 64 blocks) are enforced at
the start over the principal's rows on the body (`body_admits`, refused `CHART_FULL`), never at the
finish, so two concurrent rigs may overshoot by one act's worth. A field row carries its sharpest
`footing` (a scan never dulls it), not a "fidelity". A pocket cell's byte holds its tier in the low
two bits (`TIER_MASK`, `TIER_R2` 2) and in bit 2 `PIT_SEEN`, reserved for SM.7 and written by
nobody, every other bit refused by the row's check: the judge found the readers comparing the whole
byte, and the lead's fix round gave it this layout, so a pit bit survives a re-sounding. Each
provenance ring holds the four newest stamps, oldest first, whatever order finishes land in (the
round). `+tv/forget` writes by its own action, `tacsim_chart_forget`; it also prunes a coverage-free
budget-keeper row (below) whose IC hour is over, a bare forget of a site with no coverage answers
`FORGET_NOTHING` whatever the keeper's hour, and a row that cannot be read answers "Your chart of
{what} cannot be read right now; nothing changes." (`CHART_UNREAD`) and is never forgotten blind
(all three the round's). The status line is §1 beat 4's note.

**As built (SM.7):** a core writes `PIT_SEEN`: `chart::write_core` sets the pocket's cell to
`TIER_R2 | PIT_SEEN`, keeping its other bits, and the cell reads as sounded; a byte holding
`PIT_SEEN` with no tier is refused by the row's check and never reads sounded (pinned by the fix
round). The provenance enum gains `Instrument::Core` (serde `core`), stamped `Local { Core, at:
due_ic, Hand }`; a core touches neither last stand, so it never feeds the same-cell repeat rule. The
mine preview's blind default family is the one named, else the richest family of the operator's own
R1 row of the field, else metals (R6, an owner-critical default the lead accepted, standing until
the owner rules).

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
  each axis, a whole field from its middle: barren, lean, fair, rich per family, and its pit state. A
  sounding is **heard site-wide** as a bearing and a range band by every party on the site (the
  Epistemic Engine's rule that active perception is observable; Q-20-9), at most one line per sounder
  per IC hour and otherwise a transient mark on the site map's row of other rigs (`+tv/quiet` drops
  the lines). `+tv/sound quiet` reads eight cells and is heard by nobody, at the same cost — the
  decision beat 6 names. A scan is passive. Mining a pocket also writes its R2: the core is a reading.

**As built (SM.1):** the reading rule carries a version, `READING_RULE_VERSION` 1, which SM.4a's chart
rows pin. On regolith a barren field reads barren–lean and a rich one fair–rich; otherwise one keyed
draw at level 3 per (field, family) chooses the band's side, so a repeat never narrows and the band's
position does not tell the truth.

**As built (SM.4a):** reading rule v1 is pinned literally by
`tests/geology.rs::sm4a_reading_rule_v1_is_pinned_literally` (provinces, fields, pockets, the word
ranges and the rule's legs, on a fixed test table), so a change to any literal is a new rule
version, never a re-bless. **Reach**: a unit is in reach when its centre lies within the radius of
the stand cell's centre on both axes (doubled cells), so a scan reaches exactly nine fields, a
sounding 64 pockets and a quiet one 16, from any stand. **Sight**: a field is seen when the atlas
line of sight from the 2.5 m mast to a mast's height over the field's centre cell is clear or
partial, the stand's own field always seen; a scan from ice still costs and writes coverage (the
lead's ruling 5). The scan preview also names the hidden count ("; 2 hidden from the mast") and,
from regolith or ice, the nearest sharper footing within the scan's reach, by ring from the
north-west. A field read as a band prints the union of the band's pocket counts ("C-NW fair–rich
metals (5–16 pockets)"), so the count never excludes the truth (the lead confirmed it; §10).

**As built (SM.4a): the heard sounding.** The budget is per (sounder, **site**), stamped on the
sounder's chart row (`heard_ic`) by the finish and decided authoritatively, and it survives
`+tv/forget` as a coverage-free budget-keeper row inside the hour. The listeners are the operators
of every open excursion on the site and the helm holders of programme hulls docked on its pad, minus
the sounder and anyone quiet, **connected now only, never queued**. The line is "A sounding thumps
through the ground from the {bearing}, {band}.", the bands under 500 m, 500 m to 1 km and over 1 km,
and "right under you" on the same cell; it is sent only once the in-memory projection write that
advances `told` has succeeded, before that write is persisted, so a crash between the line and the
persist may repeat one heard line once after a restart. The
per-(sounder, site) budget surviving a forget (SM.4a's D7) and delivery to connected listeners only
(D8) are the lead's owner-critical defaults, standing until the owner rules; the site map's
transient mark is SM.4b's. `+tv/quiet [on|off]` is a preference document,
`tacsim:tvpref:<character>`, written from `+tv`'s authoritative bracket: not an act, and with no
`go`.

**As built (SM.4b):** the site map's transient mark landed on its row of other rigs: it reads the
other rig's last outcome only — a `Done` loud sounding on this site inside the IC hour — so the
sounder's next act clears it before the hour ends.

Repeating an instrument from the same cell is refused at no cost: "You learn nothing new here." —
industry's "a re-survey cannot narrow" (`mush-industry/src/commands.rs:379-433`) turned into play; the
only ways to narrow are a finer tier, better footing or a closer stand. **Provenance** is the chart's
own enum, modelled on `mush-propagation`'s but never changing it (`record.rs:663-677`, a side
projection that gates nothing): `ChartProvenance::Local { instrument, at, via: Hand | Plan }` or
`Reported { source, at, via: Gift | Sale | Salvage }`; only a `Local` row made by hand, at R1 or
finer — a scan, a sounding or a core — counts as discovery evidence for a stake, and only a hand-made
sounding the site heard, stamped inside the block's current tenure epoch (§9.4), for a contest; a row
a plan's instruments wrote while the operator was away is the character's own reading for every other
use and never priority (§8). Selling or giving a chart is phase 2 (a market
`Representation::Entitlement` listing delivered by a `DeliveryProvider`,
`mush-market/src/record.rs:132-133`, `module.rs:86-105`, with the sale as provenance). A TV's log ring
is its black box; salvaging it yields `Reported { via: Salvage }`: stolen knowledge, never stolen
discovery. Knowledge needs no tenure: a rig may drive onto, scan and sound any block, claimed or not.

**As built (SM.4a):** "You learn nothing new here." is refused when the same instrument class (a
scan; a sounding, loud or quiet) was last used from this very cell by this principal on this site
**and** the act would add nothing (D4); the same instrument from any other cell always runs, costs
its charge and stamps fresh provenance, which SM.10's in-epoch contest evidence reads. Provenance is
`ChartProvenance::Local { instrument: Scan | Sound | QuietSound, at, via: Hand | Plan }`, and only
`Hand` is written (`via: Plan` is SM.11's; `Reported` is not built: the enum has only `Local`). The
last stands the repeat
rule reads follow whichever finish landed last, not the newest stamp (the round's N2, a residual: at
worst an extra charged act that adds nothing, or a refusal at no cost). No licence and no tenure
gate a scan, a sounding, a forget or `+tv/quiet` (the lead's ruling 9).

**As built (SM.10):** the stake's evidence (`surface::tenure::evidence_at`) is the earliest
hand-made stamp — `Scan`, `Sound`, `QuietSound` or `Core`, `via: Hand` — in the actor's own
provenance rings for the block's four fields and its pocket block, plus each row's `first_local_at`
while its ring holds no `Plan` entry, so the discovery survives the ring's four-entry eviction; a
heard or plan-made reading never qualifies. The contest's evidence (`heard_before`) is the earliest
loud `Sound` by hand in the pocket ring only, from the block's last opening to the holder's stamp
(§9.4), so the ring's four entries can evict a challenger's early loud sounding (R3, §16). SM.11
must revisit the `first_local_at` clause once `via: Plan` stamps exist (§13's SM.11 row).

**As built (SM.11):** `via: Plan` is written: `write_scan`, `write_sound` and `write_core` take the
`via`, and a plan's sounding or core stamps `Local { …, via: Plan }` (§8's note). The SM.10 note's
`first_local_at` clause is replaced by each row's `first_hand_at` (`FieldRow.first_hand_at`,
`PocketBlock.first_hand_at`) — the earliest hand stamp ever written to the row, which a plan's stamp
never sets — and the provenance ring's trim evicts the oldest `Plan` entry before any `Hand` entry,
so a plan-made reading never qualifies a stake and never pushes a hand stamp out; `evidence_at`
reads the rings' hand entries plus `first_hand_at` only. `heard_before` reads hand soundings only,
so a plan's loud sounding is heard on the site but is never contest evidence.

**As built (SM.11r):** a row written before SM.11 carries no `first_hand_at`, and every stamp written
before SM.11 was a hand's, so its first stamp, `first_local_at`, is its discovery. `evidence_at`
reads a row's first hand stamp through `chart::first_hand_of` — `first_hand_at`, or `first_local_at`
for a row whose ring is not empty and holds hand entries only (a plan-first row always keeps a
`Plan` entry, so this never reads a plan's stamp) — and the chart's next stamp of such a row
backfills `first_hand_at` from it before anything else, so neither a later hand stamp nor a plan
stamp can lose the discovery (doc 20a §10, SM.11r-1; the owner's external review of 2026-10-23). No
world holds such a row today; the rule closes the gap for any that does.

**As built (SM.12b): the seeded reading.** The house's survey of Occator Rim's four held blocks
(§9.6's note) is written by `chart::write_seed`: an R1 row at `Footing::Rock`, stamped `via: Plan`,
setting no `scan_stand` and adding no `Via::Seed`, so it is never stake evidence and is the first
evicted by the provenance ring's trim.

**Fog, by construction.** Every ore glyph on every surface is the viewer's own row. "You have no
reading there." is byte-identical for unknown, uncovered and barren-but-unsounded (the posture of
`commands.rs:52-56`). Nothing lists "nearest ore". Public facts are said plainly — "That block is
under claim.", "Your licence lapsed at 05-21 14:00.", "The hopper holds 2 t more." — and hidden truth is
never said. Industry's `SurveyBands` (lifetime cap of eight surveyors per deposit, `record.rs:52`) are
not used for cell knowledge; a block's `grade_class` and `reserve_class` are recorded at opening for
the Registry's folio.

**As built (SM.4a, SM.5):** "You have no reading there." (`NO_READING`) is byte-identical for an
unknown site, a row of another site or rule, an uncovered field and a barren pocket never sounded.
The licence line is `LICENCE_LAPSED`, "Your licence lapsed at {lapses}. +licence/buy takes a new one
from today.", with `ic_stamp`'s `MM-DD HH:MM` (the licence's player-facing texts and the stamp's form
are SM.5's decision 22, an owner-critical default the lead took, standing until the owner rules);
the lapse instant is the expiry plus the grace (bought at `t0`: in force to `t0 + 168 d`, in grace to
`t0 + 171 d`, lapsed from then, so IC day 172 refuses).

---

## 6. Mining — D-20-9

**The act.** `+tv/mine [family] [cycles]` (the owner's word; `+tv/drill` an alias; Q-20-8) previews
first — **from the operator's own R2 row of the pocket, or a public pit, never the ground**: the
family (on a pocket of two the one named, else the family of the operator's reading with the most
tonnes, said in the preview and frozen into the run), the pocket's class, each cycle's tonnes from
the remaining tonnage, the run's IC time (1,800 IC s a cycle on regolith or ice, 3,600 on rock:
terrain sets the pace, the pocket sets the tonnes), the hopper before and after the run, the hold's
unreserved room after it, the charge after the run and after the trip home — and `+tv/mine go`
commits exactly that run. **The commitment is bounded by construction, never by a repeat and never
by a clamp**: with no number it is this pocket out, the hopper full or the hold's room used up,
whichever comes first (at most eight cycles, by the 24 t cap); a named count that does not fit
**refuses and names the count that does** ("4 cycles do not fit: the hopper is full after the third
— `+tv/mine 3`, or `+tv/mine` with no number"). On a pocket the operator has no reading of, the
preview says "No reading here: the first cycle will tell." and the run is one **blind cycle**, whose
completion writes the R2 and prints the class — the core is a reading, paid for like any instrument,
so the preview is never a free sounding — and a blind cycle needs **3 t of room** in the hopper and
in the hold's unreserved share, the most any class yields, so that fitting or refusing never tells
the class ("3 t of room needed to core an unread pocket", a public fact). **No `until=full`, no repeat
across pockets, no auto-fire** for a seated pilot; `+tv/mine stop` lets the running cycle complete and
yield and drops the cycles after it at no cost. Repetition across pockets belongs to the plan (§8)
and, in the drone phase, to sealed minds.

**As built (SM.7):** the preview reads the operator's own row only: on a read pocket it names the
family with the most tonnes left, then the richer class, and prints each cycle's tonnes from the
cursor's remainder; on an unread one it is the blind core, "3 t of room needed to core an unread
pocket" its only room line, the room `mine::blind_room_t` — the richest yield of the drill row, 3 t
today — derived and never a constant. A public pit adds one clause to the blind preview
(`PIT_WORKED`, `PIT_WORKED_OUT`, the map's own `pit_at` rule) and never refuses nor changes the run
(R16); `+tv/mine stop` is committed without `go` (R7). Both are owner-critical defaults the lead
accepted, standing until the owner rules; the blind core's family is §5's note. A count of one reads
"1 cycle", and a rig in the bay is refused with no implied launch (`MINE_IN_BAY`).

**The start** is an authoritative submission as the operator (`tacsim_surface_mine`), its
`project_offer` holding the world and passing server-authored fixed parameters: the TV stands in the
pocket and is idle; the operator's licence account is Active; **tenure allows it** — the block is
commons, or open with no live claim under the site's issuer, or `extraction_authorized`
(`mush-industry/src/licence.rs:156-232`, called pure over the block deposit's own claim index) answers
Ok for a claim **whose issuer is the site's issuer**; the pocket's sidecar row shows the lock free or
held by this excursion; **the hopper admits the run and so does the hold**: `capacity::admits` over
the hopper compartment (`port/capacity.rs:214`) and over the hull's hold plan with the hold's
**unreserved room** — its room by `max_more` less the ore already in every hopper aboard the hull and
less every outstanding, uncompleted cycle's yield of every TV of the hull, so a hopper never carries
what the hold cannot take and `+tv/recover` always unloads everything — refused at start, never
clamped at completion; the charge covers the run and the trip home. It writes the lock on the sidecar
row and, on the excursion's state family, the cycle `{pocket, family, k, cycles, yield, due_ic}` and
the **charge reservation**, once each.

**As built (SM.3b):** every act's start is the one action `tacsim_surface_act` (`act` a `Choice`)
and every finish is `tacsim_surface_done`, submitted by the tick as the operator; mining becomes
`act = mine` in SM.7, with no second start action (SM.3b's decision 25, the lead's default standing
until the owner rules).

**As built (SM.7):** the start is `act = mine` on `tacsim_surface_act`, and `+tv/mine stop` is
`act = mine_stop` on the same action, keyed `…/{ordinal}/stop`: no third action, so SM.3b's decision
25 stands. The preview is the command's `mine_plan`; `go` files a RAM mine order (SM.4b's drive orders
generalised to `ActOrders`), which the offer re-plans at landing and requires equal
(`MINE_CHANGED`), so `go` commits exactly the preview or nothing. The plan's gates run in order: the
stand, the lease (`lease_gate`, shared with the launch), the pocket, its footing, the licence
(`licence_gate`'s rule, in force or in grace, read through `licence_standing` by `mine_licence` so
that the lapse instant is carried as a fixed parameter), the tenure (§9.2's note), the record and
the lock (§6's and §3.4's notes), the operator's reading, then the fit. The fit is
`mine::fit`, naming the count that fits in every measure — mass and packed volume, so §19 item 1's
promise holds by volume too — against the hopper's room and `port::ship::hold_unreserved`, the one
function the map's `hold.room_t` reads, now counting every outstanding cycle of every rig of the
hull; since the fix round `mine::fits` calls `capacity::fits_in` rather than restating its rule. The
start's prepare writes the run into the record's **cycle slot** (`RigRecord.run`, beside the act
slot) and locks the pocket's cursor rows in one bundle, the run's charge and its trip home reserved;
a forfeit or ended lease bars the start (R15, §8's note). The unreserved room is read by the mining
plan and the map only: a market purchase still admits against the hold's limit less its load, so
cargo bought mid-run can leave a hopper the hold cannot take, which SM.8 must answer.

**As built (SM.8):** "`+tv/recover` always unloads everything" holds by construction for an honest
fill, and SM.7's open question is answered at the unload, no other hold writer changed: the unload's
own room is `hold_unreserved_with` — SM.7's one hold-room function, now over `capacity::{room_less,
take_room}`, with the unloading rig left out — re-checked at its start, and the finish admits
against the hold's limit less its load. A hopper the hold cannot take whole (a hold staff have
shrunk, say) unloads what fits and says what stays (`UNLOAD_STAYS`), and the recover is never
refused for it; a hold with no room at all makes the queued unload a 0 s act told once
(`UNLOADED_NONE`; `UNLOADED_NONE_EMPTY`, since the lead's fix round, when the hopper read empty),
never a stuck queue (R7, an owner-critical default the lead accepted, standing until the owner
rules).

**The completion** is the surface tick's one bundle at `due_ic`, as the operator:
`inventory.transfer(block deposit pool → hopper pool, the family's ore good, yield)` on tacsim's
delegated inventory families (`composition/mod.rs:510-515`; `mush-inventory/src/pool.rs:468-506`), the
sidecar's `drawn += yield`, the next cycle or the lock's release, the charge spent against its
reservation, and a **receipt** keyed `(deposit, pocket, k)` as the replay guard; the tick then writes
the excursion log, the chart's R2 row and the operator's line. **Nothing is minted per cycle** and no
lot is created: the reserve was credited once at opening, so conservation *is* the transfer, the
hopper never grows toward the 512-lot cap, and there is no second dissolve bundle. A bounded per-site
ring publishes the cycle as the fact `tacsim.surface.mined` for contracts and commissions (not
`industry.extraction.cycled`, `mush-industry/src/record.rs:88-89`, which is written on industry's own
families, which no owner delegates; only inventory delegates, `tacsim/src/composition/mod.rs:229`).
An `IndustryHandleCell` exported by `mush-industry` (the `InventoryHandleCell` shape) gives the
composed provider read access to deposits and claim indexes; it writes none of them.

**As built (SM.7):** each cycle lands at its own due through the one finish, `tacsim_surface_done`
with its optional `cycle` parameter: one bundle moves the family's ore good from the block vault to
the hopper by `InventoryFamilies::transfer`, adds the yield to the cursor's `drawn_t`, spends that
cycle's charge, keeps or releases the lock, and re-arms the next cycle or closes the run. The
receipt is the finish's idempotency key `tacsim.surface/{tv}/{ordinal}/cycle/{k}` over `(tv, run
ordinal, k)`, not `(deposit, pocket, k)`: the run froze its deposit and pocket, and a second run of
the same pocket restarts `k`; the prepare's `(ordinal, k)` check is the second guard. A short vault
or a retried finish is refused `Unworked`, nothing staged, and closes the run. The chart's R2 row is
written in the same bundle by `chart::write_core` (§5's note), which creates the operator's row with
the run's body when they have none. The ring is `tacsim:mined:{site}` (`MinedRing`, 64 facts,
inserted in `(at_ic, tv, ordinal, k)` order, never doubled), written by the tick's tell only after
the projection's write and only for a cycle that moved at least a tonne — a barren core is never
published — and read by nothing yet.

**Yield** is the pocket's class — lean 1 t, fair 2 t, rich 3 t a cycle — until the pocket's tonnage
is drawn, the last cycle yielding the remainder; it never rises, so D-E7-6 holds. Grade is spent here
as tonnes (D-20-10): pools stay gradeless, no bid is graded, and the dissolve-then-crystallise
laundering hole never opens. **Two rigs on one pocket**: the lock refuses the second with "Another rig
is working that pocket." — a public fact, since units on the site are drawn for everyone.

**As built (SM.7):** the lock is one cursor row per family with ore in the pocket, at most two
(`MAX_LOCK_ROWS`), and the start also refuses while any other rig's run record stands on the pocket,
so an all-barren pocket, which has no row, refuses "Another rig is working that pocket."
byte-identically (R4). A lock is stale, and taken, unless its holder's record has a run of that
ordinal on that site whose lock rows hold this very row (the deposit and the pocket): the fix round
matched it by its row, where the judged tree matched the site and the pocket's number but not its block. Yield and pace are
the drill row's (§4.1's note).

**During a run** the TV is held to the pocket, and the pilot is not: a scan or a sounding from the
new stand, the next drive's preview, the registry, the chart and the second rig are all legal. A
45-minute session is some thirty cycles and a decision at every pocket, with the next one planned
while the drill runs.

**As built (SM.7):** during a run only a scan or a sounding may start on the drilling rig
(`run_refusal`), and a drive previews but refuses `go`. Since the fix round a drilling rig is busy
(`RigFlight::Drilling`) to the departure gate, the lease return and the repossession: a departure
answers `TV_BUSY` "drilling C117" with `RUN_STOP_REMEDY` ("+tv/{n} mine stop ends the run after this
cycle."), ahead of `TV_OUT` (§4.3). The lease return's `RIG_BUSY` names the running cycle's due as
"done {when}", not the run's end (the round's N1, a residual).

---

## 7. Offload and sell — D-20-10

### 7.1 TV to ship

`+tv/recover` is one command that files two tick acts with one preview: a frozen leg to the ramp
cell and, at its due instant, the placement `Interior { bay, slot }` (`mush-sim/src/window.rs:325`);
then the unload. It cannot be one bundle — placement is a legacy slot `prepare_authoritative` cannot
reach (`mush-sim/src/journey.rs:11-20`, RESOLUTION A) — so the unload is its own act, `transfer_marked`
from the hopper pools to the hold pool (`mush-inventory/src/pool.rs:508-568`), its quantity previewed
as `min(hopper, max_more(hold plan))` — the whole hopper, by the hold-room gate of §6 — and re-checked
at prepare, the marker derived from `(tv, excursion, ordinal)`; `+tv/unload [qty]` stays for a partial
unload and a TV already in the bay. The intermediate state — TV in the bay, ore in the hopper — is
durable and the departure refusal `HOPPER_LOADED` reads it. **The capacity plan is generalised**, not
the sale predicate: `plan_of(&LogisticsDef)` (`port/market.rs:392-400`) and `capacity::admits`/
`max_more` are read over any unit's stowage block, the hopper's included, so a hopper and a Dray-P
hold refuse an overfill identically, while `governed_hold` (`market.rs:82-97`) stays the
programme-hull predicate the sale rule and `CoopProceeds` read.

**As built (SM.8):** "its own act" is `act = unload` on SM.3b's one start and one finish
(`tacsim_surface_act`, `tacsim_surface_done`), no third action. A recover's `Done` with ore in the
hopper writes a durable `RigRecord.queued` in its own bundle — a third slot beside the act and the
run — and the surface tick starts it as the operator on a later fire, under
`tacsim.surface/{tv}/{ordinal}/queued` with `how = queued`; `+tv[/<n>]/unload [<t>] [go]` files
one by hand with `how = whole` (the whole hopper up to the hold's unreserved room) or `how = part`
with `qty` (`<t>` tonnes, 1 to 99, of the first ore in id order: R10, the lead's ruling), bound to
its RAM order (`ActOrder::Unload`) by `how` and `qty`, the queued start to the record's queue, each
re-checked at the prepare (`UNLOAD_CHANGED`). A part past what can move is refused, never clamped,
in three words since the lead's fix round: `UNLOAD_NO_FIT` when the hold's room binds,
`UNLOAD_NO_STOCK` (`unload_no_stock`: "Rig {n}'s hopper holds only {fit} t of {ore}: …") when the
hopper's own stock does, and `UNLOAD_NO_ROOM` for a fit of 0 (`REFUSAL_CODES` 56); `{ore}` is
SM.7's `ore_words` ("ice ore"), not the commodity label. The finish is ONE inventory-owned
`transfer_many_marked`, not `transfer_marked`, because two `transfer_marked` calls cannot share a
bundle: one `inventory.transferred` marker a good, its id derived from `(tv, excursion|null,
ordinal, good)` at the act's due, and the hopper vault's marker ring written once. A rig gone from
the bay, a changed hopper, a full marker ring or a hold that will not take the load each refuse with
their own code (`LeftBay`, `HopperChanged`, `MarkersFull`, `HoldRefused`) and move nothing:
`HoldRefused` never overfills (R8). "`plan_of(&LogisticsDef)` … generalised" is `stowage_plan`,
`plan_compartments`, `plan_goods` and `plan_check`, `PortCapacity::check` now deserialise +
`plan_check`, while `PortCapacity::plan` and `governed_hold` are unchanged (R12), so a hopper never
gets a market plan; the preview's room is `hold_unreserved_with`, not `max_more` over the plan.
The unload works only on a site's pad (R5); the hand unload needs the operator aboard and the
queued one does not, the recover's `go` having committed it aboard (R6). R5, R6 and
R8 are owner-critical defaults the lead accepted, standing until the owner rules. A line naming
several goods joins them with " and " since the lead's fix round ("8 t of ice ore, run-of-mine and
4 t of metal ore, run-of-mine").

### 7.2 Ship to port

Only the **governed hold** sells (`HOLD_ONLY_SOURCE`, `mush-market/src/trade.rs:1836-1844`): the
hopper is refused twice, by that rule and by `may_take` (its owner is the `tacsim.vehicles` system
principal). The Starflight shape — offload to the ship, then sell — is the law. `+market/sell
<listing>=<qty>` at Ceres, or a queued `sell` in the captain's queue, into the ore intakes below; the
sell preview and `+market` print the bid's unfilled quantity, and `sell … all` sells what the bid
takes and leaves the rest in the hold. The Co-op's share applies with one amendment (§7.4).

**As built (SM.5):** `+market/sell <listing>` with no `=` is the preview — what the house's standing
bid still wants and what you hold where the sale would come from, then on a second line what selling
all of it would take and pay; it sells nothing — and `+market/sell <listing>=all` sells what the bid
takes and leaves the rest, refused `NOT_BUYING` on an exhausted bid. The listing line's `wanted N ×`
is the unfilled intake, pinned and not re-worded; the queued `sell <good> all` is unchanged.

**As built (SM.8):** pinned as written, no market code changed: the hopper is refused at sale by
`may_take` (`NO_SUCH_LISTING`) and by `HOLD_ONLY_SOURCE` (U12). The story on `build_engine` at rate
24 sells the unloaded hold at the Exchange's bid with no staff act at all (§13's SM.8 row).

### 7.3 The ore goods — Q-20-1's default

Mined matter is **raw ore, a good of its own per family**, bought only at Ceres and milled into the
concentrate and water the trade run sells. The alternative — minting the existing `metals` and
`volatiles` rows — makes mine-then-haul-to-Kasei net about 17,000 cr a leg (21,600 less the share,
the fuel and the stores) against the trader's 5,390 on the same passage (doc 18 §4.1), so every
charter stops buying at the Exchange; that is the reason this fork was the owner's. Goods table
**version 3** (doc 20 cut SM.3a; doc 19's A.8 took version 2) adds four rows under a new stowage tag
`stow.ore` (the locker rule passes because four goods share the tag, `definitions/goods.rs:384-386`);
each is the commodity of its family's block deposits:

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

**As built (SM.3a):** the four ore rows landed in goods v3 (twelve rows), each class `trade`, unit
`t`, at the figures above with `reference_major` 70 / 35 / 50 / 600; their notes say "drilled from a
Registry site", which covers Rheasilvia on Vesta as well as Ceres. They carry no listing, vendor or
bid yet: those are SM.5's, on lane v11.

**As built (SM.5):** lane **v11** carries the bids at the figures above, each posted under its
house's own name: the Exchange's `metal_ore` 70 (standing cap 240 t, drip 40), `carbon_ore` 50 (120,
20) and `strategic_ore` 600 (12, 2) after its alloy bid, and Occator's `ice_ore` 35 (240, 40); only
Ceres bids ore. Occator's capital rises by one appended tranche, `PortOrgDef.raises: [250000]`,
minted as its own tranche through `capital_schedule`: the base `capital_major` stays 50,000 and is
never re-capitalized, and an edited, dropped or reordered raise is refused as drift. That amends
D-18-27's "no top-ups" for Occator's appended raise alone (§18).

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

**As built (SM.5):** the drip is `drip_step` on the port tick's stock arm, after the ask step and
before `bid_step` (which skips refresh rows), one act a fire as the clerk: withdraw, then post
through `wanted_spec`, rewriting the row's listing, `expires` and `posted_ic`. Beyond the boundary
re-post it also renews a standing intake at its standing quantity once its deadline is within one
real day (`DRIP_RENEW_SECS`), so an untouched intake never ages to the 90-day TTL; holds at most one
banked drip for a row that waited at its cap (`posted_ic` not advanced); re-posts smaller when the
house's funds bind and leaves the listing alone at a target of 0; brings a lapsed row back at one
drip — a pack apply's re-post too, through the same `lapsed_drip` rule, since the lead's fix round
(and since RK-1 the apply's revision of a standing row, through `drip_target` within the house's
funds; an unfunded house's lapsed row it cannot re-post records a changed authored price; the
apply's first post of an unrecorded row stays the full intake, since a world's first apply runs
before its houses' capital settles);
waits for the recorded deadline of a row withdrawn by hand; marks its own withdraw-then-refused post
lapsed; and passes a row refused having moved nothing over for the rest of its IC day, so one
house's standing refusal never stalls the other intakes. Every re-post churns the bid's listing id
(`help trade` says so). The listing-id churn (SM.5's decision 19) and the lapsed row's return at one
drip (decision 20) are the lead's owner-critical defaults, standing until the owner rules.

**As built (SM.12a):** "the capital clause binds Occator in about nine real days and the Exchange in
about a month unless the mill's resale refunds it, which is the point" does not hold. The mill's
resale is bounded by the trade run's draw — at most about 16,000 cr a real day at the Exchange,
against about 120,000 cr of full drip — so at full drip the Exchange's intakes close on capital in
about 33 real days without the mill and about 38 with it, while Occator nearly breaks even (32,000
against 33,600): the drip, its caps and the capital size the intakes, not the mill (O-1's
consequence; O-1 is the owner's provisional ruling of 2026-10-24).

**The mill** is P5's supply half (doc 18 §5's post-loop list): a receipted port arm turns the house's
storefront ore pool into concentrate at the ratio and raises the ask's quantity by the proven
surplus through a new market `restock_ask` op; the market still never mints
(`mush-market/tests/e7r2.rs:305-336`). The Exchange's finite 4,000 t of concentrate gains a
replenishing source and the loop closes: miner, Exchange, trader, Kasei.

**As built (SM.12a):** lane **v14** authors two mills, `VendorDef.mill` rows: the Exchange's
`metal_ore` into `metals` and Occator's `ice_ore` into `volatiles`, both 5 : 2, 20 t a batch, 40 t
of ore an IC day (O-1, the owner's provisional ruling). "A receipted port arm … raises the ask's
quantity by the proven surplus" is **two writes**, one act a fire on the Ceres clerk's stock arm
after the intake drip and before the bids, the restock before the batch. **The batch**
(`tacsim_port_mill`) debits `batch_t` of ore and credits `batch_t / 5 × 2` of product at the one
storefront, with the mill's `MillLedger` (`tacsim.port_mill`: totals, the IC day's tally, a
16-batch tail, the ordinal as the replay guard) in one bundle; no marker, and the product enters
with no tranche and no lot lineage (O-7b, the owner's provisional ruling; doc 18's D-18-26 amended
for it). **The restock**, on a later fire, is a normal-lane `mush_market::restock_ask` — a live
listing, `may_take` before kind, a trading vendor and org, a drawn ask, the backing (balance less
holds ≥ quantity + undrawn + units), `revision + 1`, never a mint — with the per-mill
`VendorRow.mill_listed` counter in the same outbox. The restock waits while a Draw is in flight on
the pool (a delay, never a loss: the lead's ruling); a refused act passes its mill over for the IC
day, and since SM.12a a refused list step passes its ask over for the IC day too; a sold-out ask is
relisted whole by the list step, which sets the counters; and the staff remedy for a split restock
or a removed mill row is `@market/vendor withdraw <listing>` (`docs/ops.md`). A mill's ratio and
product are fixed for its ledger's life (O-5, the owner's provisional ruling; D9). `@tacsim/port
status` gains a Mill line per mill (ungated) and the Listings line nets the units milled and not yet
listed.

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

**As built (SM.5):** `reference_minor` is a second programme map beside `reference`, read first by
`CoopProceeds` (no memo, no warning), so every reader of `reference` keeps its type; a good sits in
at most one of the two, and the validator refuses a fixed reference for any good some vendor of any
port of the pack asks. Lane v11 authors the four figures above: 10 t of metal ore sold at 70.00cr
against the fixed 35.00cr gives the Trust 87.50cr.

**As built (SM.8):** pinned as written: the story's sale of its 12 t pays the Trust
`margin_and_share(price, reference_minor, 12, 250)`, a quarter of the margin over the fixed 35.00cr
at whatever the bid reads at the sale (at the authored 70.00cr bid that is 105.00cr by arithmetic;
the story reads the bid's price and asserts the share by the formula, never the figure).

---

## 8. The clock — D-20-11

**The surface tick** `tacsim.surface`: Coarse, 1 s, the journey tick's shape
(`mush-sim/src/journey.rs:69-79`): each fire visits at most 64 excursions whose `due_ic ≤ now`,
oldest due first, works at most 16, and acts **as each excursion's operator through
`Fire::as_principal`** (`port_tick.rs:1043-1059`), whose `arbitrate` admits one act per principal
per fire, so an operator's two TVs complete on two successive fires — the TV's due instant is the
authority, the fire only the check. Acts: launch done, leg arrival (the pose is written once at
arrival; positions between are pure interpolation from the leg record), scan done, sounding done,
cycle complete, recover done, unload done, plan step. A drive's A* runs **once per command** over the
composite with a budget of 65,536 cells, a whole site, passed to the atlas's `path`
(`mush-atlas/src/physics.rs:614-622`; the atlas default `path_max_cells` is 4,096, `physics.rs:92`;
the arena already raises its own to 65,536), never per tick, and the path is frozen to at most 32
waypoints with cumulative IC seconds. The engine's submit queue (64 actors,
`mush-core/src/engine.rs:732`) is shared, so the tick stops visiting when it holds sixteen acts. Caps:
32 excursions a site, 1,024 world-wide, **four per operator** — the Compact's span-of-control lever;
a Dray-P carries two, so a live hour may be one TV and a 75 s rhythm, or two staggered TVs and a
decision every forty seconds.

**As built (SM.3b):** a fire reads the whole family (at most 1,024 records, in pages of 256) and also
visits records whose last outcome is untold; candidates are ordered by `(due_ic or last.at_ic, tv)`,
and an act in back-off is dropped from the window of 64 so it never starves newer acts. The caps
count open excursions in the start's prepare, over every lessor pool's rigs.

**As built (SM.4b):** the path is frozen to at most 32 waypoints **within a display tolerance of at
most 80 m** — an integer Douglas–Peucker at 1, 2, 4 then 8 cells, the first keeping at most 32
points winning — while the time and the charge are the exact path's, every kept waypoint carrying
the exact path's cumulative seconds; a way needing more than 32 even at 80 m is refused
(`TOO_WINDING`), never cut, and a recover's way home is never refused for its waypoints (P4, an
owner-critical default the lead accepted, standing until the owner rules: the lane's per-cell
terrain makes "corner points" refuse nearly every drive). The census over every lane site, from the
ramp to all 64 field middles, refused none and froze every leg at the 1-cell tolerance, at most 24
waypoints. Features are not obstacles: the atlas's `path` reads cells only, the lane's craters
author `movement: false`, and `tests/driving.rs` fails the day a lane site authors a
movement-blocking feature (P5). The A* runs once a command over the composite at 65,536 cells, and
the command's whole planning measured about 30 ms in release on the worst case, corner to corner:
the lead's 20 ms threshold was crossed, the cost inside the atlas's `path` (string-keyed
per-neighbour lookups and a weak heuristic), and the lead accepted it for SM.4b with an atlas cut
owed after SM.7 (§13's SM.12 row). A drive starts and finishes on the same two actions (`act =
drive`), its leg in the act's params and projected into `tacsim:tv:<tv>` at the first fire after its
start, the rig placed once, at the leg's end, at the due instant; the start's optional `to`
parameter binds it to the drive's order, a RAM `DriveOrder` the offer fixes, so a restart loses an
unlanded order ("not planned; preview it again").

**As built (AP):** the atlas cut owed above landed as AP (`74c5801f`, an engine cut in `mush-atlas`
with no §13 row of its own, between SM.7 and SM.8): `path` resolves its locomotion's rows once per
search into a dense per-class row (`mush_atlas::LocomotionCosts`, one step rule with `move_cost`)
and steers by the octile distance priced at the cheapest passable class present on the surface —
admissible and consistent, so it returns the least cost, unchanged for every row whose passable
classes all cost at least 100, the drive's tracked among them (`wheeled`, which no production
caller searches, may now find a cheaper path the old `Chebyshev × 100` missed) — and the home field
reads the same row. On Ahuna Flank in release, corner-to-corner planning went from 28.86 ms to 3.28
ms (minimum of five; medians 29.57 → 3.46), the cached home field from 31.32 to 7.04 ms and the
exhausted flood from 28.99 to 8.31 ms; in debug 151.97 / 193.59 / 156.14 → 22.21 / 84.63 / 65.47
ms. `sm4b_the_drive_planner_is_measured` is a release gate (≤ 10 / 20 / 20 ms on the minimum of
five); the debug pair 100 / 250 / 250 ms is asserted under `TACSIM_ASSERT=1` only, never in the
push-triggered CI shard (D-H-8, D-16-19; an owner-critical default the lead accepted, standing
until the owner rules). Every lane leg's cost is unchanged — the drive's tracked row, proven
against a plain Dijkstra over `move_cost` on every lane site (`AP_SITE_LEG_COST_TOTALS`,
`AP_AHUNA_CORNER_COST` 60,720); one equal-cost tie on Ahuna froze to a different waypoint count
(the 64 ramp legs' spread 3/12/24 → 3/13/24, the most still 24). The RAM memo of the last planned
leg was not built (SM.7's one-slot `ActOrders`). The cites `physics.rs:614-622` and `physics.rs:92`
above are stale after AP: read them as `mush_atlas::physics::path` and `DEFAULT_PATH_MAX_CELLS`.

**As built (SM.12c-lane):** AP's memo, the cost lane's decision (b), was not needed at this cohort:
the surface tick's per-leg p99 was 1.2 ms (1,170 µs) in the final 30-TV month and the hand drive's
1.1 ms (1,072 µs, an upper bound), and AP's re-read in release sits nearest its line, an
exhausted-budget median of 9.43 ms against 10 ms. The tick now times itself, in RAM only and never
persisted or shown to a player: `SurfaceFireStats` gains the fire's `elapsed_us` (the warm
included), `warmed` and `warm_us` (`Warmed { Nothing, Site, Home, Failed }`), `searches` and
`search_us` (every `drive_plan` call `decide_walk` makes, yard rigs included), `peeks` and `peek_us`
(`chart::peek_site` memo misses) and the count `markers_held`; the port tick's `PortFireStats` gains
`elapsed_us` and the stock, site and dispatch arms' `ArmFireStats`. The surface fire's release p99
was 0.77 ms at ten TVs (C2's gate, under 5 ms) and 1.4 ms over the 30-TV month (§11's Tick row).

**As built (SM.9):** "positions between are pure interpolation from the leg record" now holds on
the clients too (the lead's 0z ruling, reversing the planner's timed re-pull): the TUI and the web
place the viewer's own rigs along `site.map`'s `rigs[].leg` by SM.4b's `leg_position` rule, from
`site.clock { now_ic, rate }` and `site.under` (the glyphs beneath them), re-pulling on `tv.status`
only and on no timer (§10). The frame carries `rigs[].leg` only for an act on that very site.

**Every act starts authoritatively.** A scan, a sounding, a drive, a mine run, a recover and an
unload each begin as an authoritative submission as the operator, whose `ActionTxn` writes the
excursion's **state family** `tacsim.excursion` — the current act `{kind, params, started_ic, due_ic}`,
the **charge reserved for it** (the reserve rule, §4.1, is enforced here and nowhere else, so two acts
can never spend one margin), the current cycle `{pocket, family, k, cycles, yield}` and the locked
pocket's address (the lock itself lives on the sidecar row, §3.4) — and the tick **completes** it at
`due_ic`: the placement for a drive's arrival, a launch and a recover, the chart row for a scan or a
sounding, the transfer for a cycle, the `transfer_marked` for an unload, each verified by pull the way
the executor verifies an order (a refused completion releases the lock and the charge and writes the
public line). `tacsim:tv:<obj>` is the **normal-lane projection** in the `JourneyRec` shape —
operator, slot, `ExcursionId`, state (`Bay | Out | Held | Parked`), pose, the frozen leg, the plan, a
log ring of sixteen lines, at most 4 KiB — written in the same Outbox tick batch as the placement,
the act ordinal its idempotency key; it decides nothing. Nothing is RAM-only: a restart resumes at
the next fire, overdue acts applying at their own stamps, so the post-image after a restart equals an
uninterrupted run (an acceptance test).

**As built (SM.3b):** the launch and the dump also start this way. The state family holds one
`RigRecord` per terrain vehicle, not per excursion: a monotonic act ordinal, the lazy charge
`{value_cu, at_ic, in_bay}`, the act in flight with its `reserved_cu`, the open excursion and the last
`Outcome`. The finish is itself an authoritative submission, `tacsim_surface_done`: it records `Done`,
or `Refused` (`NeverLeft`, `NotBeside`, `HopperChanged`, `SpoilGone`), from the world its offer reads
at landing, stamped at the act's own `due_ic`; a refused finish never spends charge, and the next fire
tells the operator one line (`ACT_FAILED`). A finish that does not land is retried, with a page after
8 attempts and a back-off of 60 fires. The projection reads `Bay` only with no excursion open and the
rig aboard its hull's bay.

**As built (SM.4a):** the scan and the sounding start and finish on the same two actions (the
start's `act` words are `dump`, `launch`, `recover`, `scan`, `sound` and `sound_quiet`; the test
world's Probe kind is retired). The chart row is written in `tacsim_surface_done`'s bundle beside
the rig record, stamped at `due_ic`, with `ChartProvenance::Local { instrument, at, via: Hand }`
only (`via: Plan` is SM.11's). A finish is also `Refused` with `Moved` — a scan or a sounding whose
rig was not at its frozen stand at landing: no charge spent, no chart write — and an implied launch
whose rig never left the bay is `NeverLeft` (§4.3). The finish's outcome carries `span`, what the
act read, for the tell, which prints the reading block from the operator's row restricted to that
span; a row gone, unreadable or under another rule tells `READINGS_GONE`, and a row that stands
while the ground cannot be read tells `READINGS_DARK` (§10). `+tv/forget` and `+tv/quiet` are §5's.

**As built (SM.7):** the current cycle is the record's **cycle slot**, `RigRecord.run`, beside the
act slot. The `Run` also freezes the site, body, stand, number and letter, the deposit, the good,
the block vault and hopper endpoints, `secs`, `cost_cu`, `reserve_t`, the true `class_t` and
`pocket_t` (never projected), `stop`, `lapses_ic`, the lock rows, `due_ic` and the remaining
`reserved_cu`, at most eight cycles (`MAX_RUN_CYCLES`); `reserved_total` is the act's plus the
run's, and only a scan or a sounding started later may share the rig. The tick takes the earlier of
the act's and the run's due, a tie to the run, keyed `(tv, ordinal, cycle)` with each key's back-off
its own, and tells `CYCLE_DONE` with its end clause or `CORED` with the operator's new reading. The
projection carries the run as `RunView { ordinal, k, cycles, due_ic }` — no pocket, family, yield or
class, so a blind core cannot leak — and, since the fix round, from the first fire after its start
lands rather than the first cycle's due. A landed mine order is skipped by the offer without a
re-plan and pruned from `ActOrders` by the tick's complete scan (the round). The post-image after a
restart equals an uninterrupted run, pinned for a run by M12 and M12b.

**As built (SM.8):** "unload done" is the `Unload` arm of the one finish, and the unload starts on
the same action (`act = unload`; §7.1), the finish's `transfer_many_marked` in place of the
`transfer_marked` above. The tick gains a fourth candidate reason: a queued unload on an idle record
outside its back-off, ordered by the queue's `at_ic` and submitted under its own attempts key `(tv,
ordinal, QUEUED_CYCLE)`, never a finish's retry; it places nothing. "Nothing is RAM-only" holds for
the queue — U7 and the story's restart pin the post-image after a `build_engine` restart equal to
the straight run's — while a hand `+tv/unload go`'s order is RAM like a drive's ("that act was not
planned; preview it again" after a restart). The finish's offer now also fixes the real second
`wall` beside `now` and `clock` (the market lot offer's precedent), for the marker ring, so the
finish's fixed parameters are no longer a pure function of IC time. Every start clears the queue.

**As built (SM.9):** the projection is pushed, and carries more. `write_projection` pushes the
operator's whole projection as the `tv.status` keyframe (`rig: TvRec`, retain latest) only when the
store wrote (§10). The SM.7 note's `RunView` is reversed under the lead's ruling 4: it now carries
`addr`, `good`, `stop` and `blind` — the operator's own choices — and never class, yield or tonnes,
so a blind core still cannot leak its class; the screens read it, and `RigRecord.run` is read by no
frame. `TvRec::check` bounds the address at `MAX_RUN_ADDR_BYTES` 8 with its own refusal, "a rig
projection's run address is at most 8 bytes", apart from the cycle bound's (the lead's fix round).
The projection is its current operator's (`projection_for`, SM.9's D16): a stored record another
operator or another excursion wrote is carried with its log, leg and run cleared, so the log ring
holds the open excursion only, and a recover's tell, which writes no excursion, starts a clean log
at the bay (§16).

**As built (SM.11):** `Parked` means parked where it stands: a rig on the ground with nothing in
flight whose charge cannot pay its way home — the idle return's recover refused `leg_short` (or its
way home too far), or a plan ended `Stranded` with no act since. It is told once (`STRANDED`, "…; it
is parked where it stands on {site}. Staff can bring it in.") and the idle return is retried a day
later; no staff verb brings it in yet (§16). A plan's normal park is `Bay`.

**Rights are per act.** The licence and tenure gate **mining only** (and staking and renewal, §9);
scanning, sounding, driving, recovering, unloading and dumping need the operator's control lease and
the hull settled, nothing more. A licence that lapses or a claim that is lost stops the next cycle
with a public line and never a rig's way home, an unload, or a sounding on anyone's block; a plan
whose rights lapse skips its remaining mine steps, recovers, unloads, parks and says why.

**As built (SM.3b):** "the operator's control lease" is read as the live lessee aboard the settled
hull (§4.1's note; the lead's default standing until the owner rules).

**As built (SM.5, SM.4a):** `+tv/lease` is the one act besides mining, staking and renewal that
reads the licence (§4.3); a scan, a sounding, a forget and `+tv/quiet` read neither licence nor
tenure.

**As built (SM.7):** the licence, the tenure and the lease are judged for the next cycle, never the
running one and never the way home. The licence is judged at the next cycle's own due against the
lapse instant read at landing, whether the standing reads in force or already lapsed (R9), so a late
landing after a restart decides as the uninterrupted world; the tenure and the lease are the offer's
verdict at landing, since claims carry no instant to replay (R9's residual, as is its converse: a
renewal during the gap reads in force). **A forfeit or ended lease also stops the next cycle** and
bars a start, never a recover, a drive or a dump (R15, an owner-critical default the lead accepted,
standing until the owner rules). The running cycle always yields; a refused cycle (`Moved`,
`Unworked`) spends nothing, moves nothing and closes the run with its locks released. Since the fix
round a registry read that fails at landing (an unresolved issuer, an unreadable deposit or claim, a
missing site) ends the run as `RunEnd::Dark` with its own line, `RUN_DARK` (" The registry cannot be
read here; the run at {addr} ends with this cycle."), never "That block is under claim.", and pages
once; the owner rules on its wording. The end's precedence, the round adding `Dark` to the judged
order, is `Refused > WorkedOut > Stopped > Dark > Licence > Claimed > Lease > Done`.

**Leaving it running.** `+tv/plan <addr>[,<addr>|<block>…] [family=<good>]` files a **fill plan**:
work these pockets in order — a block letter sounds its four fields and mines every pocket of the
plan's family that reads fair or better, nearest first; the family defaults to the first address's
best reading and is frozen into the plan — recover and unload at every full hopper, park when no
cycle of the next pocket fits the hold's unreserved room or the list is worked out; the compiled
closed vocabulary (scan, sound, drive, mine, recover, unload, return-on-idle) is what the preview
prints with its IC hours and `+tv/plan show` reads back. It names only pockets and blocks in the
author's own knowledge and rights, is **bounded by one landed hold and 288 IC hours** (twelve real
hours), one plan per TV, and **it never re-arms**: a human files each shift, so a player who logs in
once a day fills about one hold a day. The captain's queue gains one closed kind, `await tv
[<slot>|all] [until=<≤288 IC h>]` — `all` the default, completing when every TV of the hull is parked
or every plan has ended; a slot number when one rig's parking is the signal — so `await tv all;
depart ceres; sell metal_ore all` runs the hop and the sale after the hold fills; a `depart` is
refused while any TV of the hull is out. With no order for 24 IC hours a TV out returns and parks —
a Compact floor like the recall arm, not a machine decision, and never the queue's bound. Reconnect
notices: at most one a trip plus a summary, in the existing `tacsim:notice` ring
(`port_tick.rs:91-99`). This **extends Q-18-22** (doc 18 §8: "no repeating plans") rather than
breaking it — a fill is one fill of one hold, as a queued buy's `fill` (D-18-41) completes one order
and never loops — and records the reading of **T13** this rests on: offline ore, and the readings a
plan's instruments and cores log, are *owned-asset production and the character's own instruments on
a sortie the character dispatched* (doc 17 R-B's lance), through a plan the character signed, exactly
as the queue's offline sells are the pilot's own signed orders (D-18-4), and never a seat; a
plan-written reading carries `via: Plan` and is never discovery evidence (§5). The owner accepted
the bound (Q-20-3).

**As built (SM.11):** the fill plan is a durable record on the rig, `RigRecord.plan` (a `FillPlan`:
one a TV, at most 16 addresses compiled to at most 64 steps), filed by one `plan_file` submission
and stepped by the surface tick through the existing acts as the operator (`Fire::as_principal`,
keyed `tacsim.surface/{tv}/{ordinal}/plan/{step}/{word}`); nothing of it is a RAM order, so a
restart re-decides from the record. The compiled closed vocabulary is three targets, not seven: a
loud sounding of a field, a bare mine run of a pocket, and every fair-or-better pocket of a block (a
block letter compiles to its four fields' soundings, nearest first, then the block). The drive, the
recover, SM.8's queued unload, the park and the idle return are the interpreter's moves, decided at
each step's start and never compiled (D3), and there is no `scan` step (D2: a nameable address is
already R1 or finer, and a sounding reads finer). The preview prints the steps with a lower-bound
estimate of their IC hours and names another rig's live plan on the hull (`PLAN_SHARED`); the one
landed hold is the unreserved-room rule across every rig's outstanding steps; `+tv/plan show` reads
the record back, each step done, now, next, pending or passed over, with the tally. The rights
are re-read at every step's start, a sounding's and a drive's too, and any lapse parks the plan with
its reason. The 288 IC h bound stops new steps and never a step in flight (D7); since the lead's
fix round it also ends a plan from any wait (`End { Bound }` unless an act or a run is in flight),
so no wait outlasts it. A plan's soundings are loud (D5). A plan waits while its author is ashore or
off the hull's helm (D4, D22), told once; since the round the author shows and clears their own
plan from anywhere aboard the hull, never from ashore, and the bound's end lands as the author
wherever they stand. `+tv/plan clear` commits with no `go` (D6), two writes on two keys (D17): on a
live plan `plan_clear`, keyed `…/plan/{no}/clear/{seq}`, ends it `Cleared` and lets the step in
flight finish — since the round it carries the fixed `PLAN_CLEAR_ORDINAL` 1, so a clear retyped
across a queued unload's landing is one intent — and on an ended plan `plan_strike`, keyed
`…/plan/{no}/strike/{ordinal}`, strikes it from the record. A live plan holds its rig from every
hand start (D8, `PLAN_LIVE`), SM.8's queued unload excepted. T13's reading holds as built: every
plan-made reading carries `via: Plan` and is the character's own for every use but a stake — the
repeat rule (`repeat_refused`) treats the plan's sounding stand as theirs (P9) — and never discovery
evidence (§5's note). D2, D4, D5, D6, D7, D8, the caps and a lapse parking the plan at any step's
start are owner-critical defaults the lead accepted, standing until the owner rules (§15).

**As built (SM.11): the queue, the departure and the notices.** `await tv [<slot>|all]
[until=<IC hours>]` is one closed kind of the captain's queue (`What::AwaitTv`, wire `await_tv`;
`until` at most 288 IC h, and 288 by default), and it reads per rig: done when every rig (or the
named slot's) is in this hull's bay with no act, run, queued unload or live plan, or on the ground
with nothing in flight and its plan ended with no act since or its projection `Parked`
(`rig_parked`); a rig out by hand with an ended plan keeps it waiting. A `depart` is refused also
while any rig works a live plan, in the bay included — `TV_BUSY` "working its plan" with `+tv/<n>
plan clear` as the remedy (`PLAN_REMEDY`), by hand and from the queue (D8). The notice budget
applies to every surface line, hand-made or plan-made, while the operator is offline: an unload, a
refused act, a run ended by a right or the dark registry, a plan's parking or end, an idle return
and a stranded rig are queued as trip-level lines, and a launch, a recover, a drive, a reading, a
cycle or a core never; a connected operator hears every line, and SM.3b's offline `LAUNCHED` queue is
gone (D13). The ring is still `tacsim:notice`'s, 32 lines. The idle return is a code constant,
`IDLE_PARK_IC_SECS` (24 IC h): the tick recovers a rig left that long on the ground with nothing in
flight and no live plan, as its operator with `by: idle`, and tells `IDLE_HOME`; a way home its
charge cannot pay parks it where it stands (above). `await tv` per rig, D8's departure and D13 are
owner-critical defaults the lead accepted, standing until the owner rules (§15).

**As built (SM.11r):** "one landed hold" holds **in every measure**. The compile's hold rule, the
preview's room and the plan's end summary read the hold's unreserved room in tonnes of the plan's own
ore by mining's fit rule over every measure (`plan::room_in_tonnes_of`), as the interpreter's step
gate already did, never the mass entry alone: with bulky cargo aboard (44 t of industrial parts in a
Dray-P leave 42 t by mass but 2,000 L) a metals plan previews 4 t of room, an ice plan 2 t and is
refused, and the end line names the room left in the plan's ore. The `site.map` frame's hold line
stays the mass entry (doc 20a §10, SM.11r-2; the owner's external review of 2026-10-23).

**As built (the SM.12c-lane follow-up, 2026-10-26): a list with nothing left to mine is refused.**
The compile refuses, after it builds the steps, a list that sounds nothing new and whose every
pocket the walk could take is worked out — a named pocket, or a block's pocket the author reads fair
or better in the family — on the public worked-out pits ("Every map shows this pit worked out"), the
walk's own test (`PlanRefusal::NothingLeft`): "That list has nothing left to mine in {family}: no
pocket you read there is fair or better and unworked, and nothing is left to sound; a plan would end
at once." (the owner's wording). Such a plan used to file, preview about 0 IC h and end "its list is
worked out" at once; a list with a field still to sound still files, since a sounding may read ore
the author has not. The 30-TV month's metal plan pilots filed exactly such plans over worked-out
metal blocks and came home with nothing (§16).

**As built (SM.12b): the house re-arms, players never.** "It never re-arms" holds for every player's
plan; the NPC house's plans are re-armed (O-14, the owner's provisional ruling of 2026-10-24).
Occator's dispatcher files its rigs' fill plans through the player's own compiler, and
`yard::file_due` re-files one after `HoldFull` or `Bound` once the lane's `rearm_ic_secs` (3,600 IC
s) has passed; after `WorkedOut` only when the house's own row reads fair ore left and the ended
plan landed a cycle; and never after `Cleared`, a right, `Stranded`, `ChartFull` or `Dark` (§12.3's
note). **The bound** (SM.11's residual R3, closed): any plan already parking keeps its parking
reason — `HoldFull`, `WorkedOut`, `Claimed`, a right, `ChartFull` — when the 288 IC h bound ends it,
and this reaches players: a player's plan heading home full whose author is ashore past the bound
now ends "hold full", not "out of time"; only a Live plan ends `Bound` (`plan::bound_over`, the
brief's own literal, wider than its wording; §16).

**As built (SM.12c-lane): the plan's double search.** SM.11's residual, a plan's drive running its
A* at the tick and again at the offer, is measured and not built: about 6,600 plan-drive landings
(6,582, an upper bound) against 6,854 tick searches in the 30-TV month. The cut note leaves the lead
a sketch — the tick's searched leg reused by its own offer, keyed (rig, target, ground revision,
record ordinal), RAM only — owed to a later cut (§16).

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

**As built (SM.5):** the licence's terms are lane data, `PortDef.licence { org, fee_major,
term_ic_days, grace_ic_days }` on `ceres_port` (`{freeport_registry, 2500, 168, 3}`): one block a
lane, and every site's issuer is its org. The record is `tacsim:licence:{gateway account}`, holding
the `holder`, the `entitlement` account, `first_bought_ic`, `expires_ic`, `grace_secs`, `terms` and
the `contract`. The fee is a Public E4 contract paying the Registry, whose `on_expiry` is `Forfeit`
to the Registry; a forfeited fee is not chronicled, since `CanonEvidenceBridge` reads `Settled`
only. The grace authorizes and a buy in the grace extends from the expiry, so "bought while Active"
reads "while in force or in its grace" (SM.5's decision 16); one licence a gateway account, an alt
refused (17); no cap on buying terms ahead (18) — these three and the forfeit to the Registry are
the lead's owner-critical defaults, standing until the owner rules. **No unpaid entitlement is ever
Active**: the buy opens the entitlement account Frozen and activates it only once the fee contract
is live, so a refused buy leaves a Frozen account with no record, which the next buy reuses and
activates (the judge found the account opened Active before the fee was taken; the lead's fix round
did this). Staff must never unfreeze a Frozen entitlement that has no record (`@account/unfreeze`
clears any Frozen account): it would recreate an unpaid Active licence, which industry's own
`@extract` path accepts until SM.6 (the round's R1). The entitlement account is never closed on a
lapse, so every licence-gated act calls `licence_gate` (Ok in force and in grace) before
`extraction_authorized`: SM.7's mine start passes the record's `lapses_ic` as a fixed parameter its
prepare re-compares with the world's IC now, and industry's own `@extract` path is closed for
surface blocks only by SM.6. The instrument is registered create-only by `ensure_instruments` and
also by the port applier for the licence-block entry, so a roster without the sheet module reads the
Licence readiness line Ready (the builder's decision 1).

**As built (SM.12b): the house's licence.** Occator's dispatcher mines under a fee-less century
licence: an Active entitlement account of the org at its root scope, opened (or adopted) by the site
clerk and recorded as a `HouseLicence` on the site row, never a gateway-account record — its
existence implied by the lane's yard block and its term SM.10's `SEEDED_TERM_IC_SECS`, neither a
lane field (O-12, the lead's default standing until the owner rules). The dispatcher's
`licence_standing` reads it (Unlicensed, Suspended, InForce or Lapsed), its Suspended branch being
exactly the industry predicate's `licence_accounts` selection.

### 9.2 Blocks — Q-20-2's default, as reviewed

The unit of tenure is the **640 m block** (16 a site), each with its tenure word (§3.1): `commons`
(licence only; a shared reserve worked out by everyone, never stakeable), `open` (**workable under a
licence until someone stakes it**, then the holder's alone — the owner's review found the loop
mining open ground before any stake, and this is the rule that makes the walkthrough true and the
stake a decision: pay to keep rivals off ground you found), or `held` (an org's). A claim is a
`Deposit`-subject `ClaimRecord` over each of the block's opened deposits — at most four, so the
as-built offer filter (`provider.rs:752-761`, which honours Deposit claims only) needs no change —
Exclusive, priority by filing order, term 168 IC days, Transferable and pledgeable as titled assets
are. **Jurisdiction**: the mining gate honours only claims whose issuer is the site's issuer, which
closes the self-issued-tenure hole (`provider.rs:841-878`). At most **two live claims per account**.
`ClaimRenew` is a new industry op that extends a live claim's term **in place**, so renewals never
spend `MAX_CLAIMS_PER_SUBJECT` 8's slots (`record.rs:54`; lifetime because the claim index is
add-only, `families.rs:69-75`). **Use it or lose it**: renewal needs **50 t worked in the term**
(previewed on `+claim/renew`, shown on `+registry` as "worked 32 of 50 t") plus the term fee; a claim
worked less lapses and the block reopens, so nobody banks ground for a cycle a week. A lapsed claim
goes `AssetLifecycle::Abandoned` (non-terminal, the last owner kept) and the block's **tenure epoch**
advances; a stake on a block whose claim is Abandoned **takes that record over** — `TransferTitle`,
a new term, holder, discovery stamp and epoch — and never files a new one, so stake, lapse and
re-stake never spend the lifetime cap either. **The cooling term**: the lapsed holder, and anyone
acting for the same principal, may not stake that block again for one full term (168 IC days) from
the lapse, a fact the stake preview prints, so letting ground lapse and re-staking it with old
evidence and another fee is not a way round the worked floor.

**As built (SM.6):** the industry half landed; the player's acts are SM.10's. The tenure is one
`TenureRecord` per block place, shared by the block's ≤ 4 Deposit-subject claims; its `TenureMark`
(`Lapsed` or `Held { holder }`) records what opened the current epoch, so a lapse, a take-over or a
seizure that transfers advances the epoch **once per block event**, however the claim writes are
bundled, and two different tenure changes on one place in one bundle refuse. Epoch 0 is "since the
block opened" and a first filing writes nothing; a renewal and a plain seizure do not advance it
(D9, the brief's reading of the lead's ruling 6, which the lead confirmed). Because the mark records
only the last event, SM.10 runs every tenure event over the whole block at one term. `ClaimRenew`
extends a live claim in place (`ClaimLapsed`, `ClaimNotLive`, `TermNotLater`); a lapsed claim answers
`ClaimExpired` at `extraction_authorized` whatever its term, and a lapsed exclusive claim still
blocks a same-issuer exclusive filing with "that claim has lapsed: take it over"; the take-over refuses a live
claim (`ClaimNotLapsed`) and a cooling holder (`Cooling { until }`). The cooling term is kept as at
most four rows on the tenure record, one per holder (the later `until` kept, rows that ran out pruned
on write, the soonest-ending other holder dropped past four and logged); its "anyone acting for the
same principal" half is SM.10's. Industry's seams take durations as arguments, and the numbers' home
is tacsim's `crates/tacsim/src/surface/tenure.rs`: `CLAIM_TERM_IC_SECS`, `WORKED_FLOOR_T` 50,
`COOLING_TERM_IC_SECS`, `CONTEST_WINDOW_IC_SECS`, `MAX_LIVE_CLAIMS_PER_ACCOUNT` 2 and
`REGISTRAR_WORKED_T` 100, with no fee constant (the claim fees are SM.10's).

**As built (SM.7):** the mining gate reads tenure by block (`mine_tenure`), read only, calling
`extraction_authorized` over the claim's own deposit: commons never reads claims, any contest state
bars the block, its holder included, and a claim whose term cannot be compared counts as live, fail
closed. Until SM.10 seeds Occator's claims an authored **held** block with no live claim refuses
"That block is under claim." (R5, an owner-critical default the lead accepted, standing until the
owner rules; a held block naming no holder reads open, as `block_tenure` counts it), and a claim
past its term that nothing lapsed reads as open ground (R20, a residual for SM.10's lapse
machinery). A dark registry read refuses the start and ends a run with its own line, never the
claim line (§8's note).

**As built (SM.10):** the player's acts landed. The block record `tacsim.claims` (`BlockTenure {
claim, paper }`, keyed `{site}/{l}`) is the one public source of a block's tenure (D17), staged
inside industry's own bundle by the composed `SurfaceTenure` authority (OC11), while the epoch and
the cooling rows stay industry's `TenureRecord`; a deposit-less block moves them through industry's
block-level `tenure_lapse` and `tenure_hold`. Surface claims are `Bound` — no title, no transfer and
no pledge (OC4, D20) — so "Transferable and pledgeable as titled assets are" yields. Every tenure
event is ONE `industry_claim_tenure` act over the whole block (the lead's ruling 1). A stake on a
block whose claim lapsed takes it over (`take_over_claim` after a block-level `tenure_hold`, the
epoch advanced once, a deposit-less block too) — its lapsed claims taken over, never re-filed, any deposit with no claim filed in the same act. The worked floor reads the
cursors less the record's `worked_at_term_start` (the lead's ruling 4). The clerk lapses a due
claim, one a fire, stamped at its term end (ruling 5), and `+claim/release` is a lapse by choice
with the same cooling (ruling 6, OC6); the cooling term binds the lapsed holder, anyone who acts for
it and every character of the holder's gateway account (OC15), staff refused too while a row is
live. Seeding Occator's claims retires R5's stand-in for the lane's held blocks (§9.6; R5 stays the
fallback for an unseeded held block), and the clerk's lapse closes R20, a lapsing claim reading open
until the lapse lands. After industry's verdict the mining gate reads the block record: a live claim
held by neither the actor nor someone the actor acts for refuses "That block is under claim." on a
deposit-less block exactly as on an ore block, and an unreadable record is dark. OC4, OC6, OC11 and
OC15 are the owner-critical defaults the lead accepted, standing until the owner rules.

### 9.3 Staking

`+claim/stake` is filed **from the TV standing in the block** (the holder is the actor's own
principal in phase 1; org-held claims are phase 2) and needs your own hand-made Local reading of the
block at R1 or finer — a scan, a sounding or a core; a gift, a purchase or a plan's reading never
qualifies — an Active licence from the site's issuer, and 2,000 cr plus 2,000 a term. Its preview
prints your readings of the block, the cooling term if one binds you, and the public fact "soundings
heard on this block since it last opened, before yours: N", so the contest risk is knowable before
the money; `+claim/stake go` files it, the player's own act under the server-authored issuer, as the
licence is; the deposits were opened at the site's opening, so nothing is minted here. A second
applicant on a block with a live claim **under the site's issuer** refuses "That block is under
claim."; paper under another issuer bars nothing (§9.6). The block turns `#` on every site map and
`+registry <site>` lists it: block, holder, issuer, term, epoch, declared family, discovery stamp,
production to date (worked this term against the 50 t floor), contest state — never truth. The fee
contract is public, so a stake and a renewal are chronicle entries; a lapse pays nothing, so SM.10
commits its own canon draft for it.

**As built (SM.10):** `+claim/stake` is one act over the whole block from a rig standing in it (the
lead's ruling 1; the stake from the ground only, OC8), its holder the actor, with an optional
declared family that is public on the registry (OC7); nothing printed reads the block's deposit
count, so a barren and an ore-bearing (block, family) answer with the same line and fee (§3.4). It
reads `licence_gate` first (in force or in grace). The fees are lane v12 data on the licence block
(`claim_stake_major`, `claim_term_major`, 2,000 each; OC1), the preview reading "2000.00cr now and
2000.00cr a term (4000.00cr today)": one Public fee contract in the licence's shape, escrowed by the
pilot, accepted by the site clerk for the Registry once the act lands, and read by its whole terms
(`fee_terms_hold`). The heard count scans at most 512 chart rows and reads "at least N" past them
(OC12). The two-claim cap counts the account's live claims over the lane's sites × A–P plus its
young `Offered` orders, re-counted inside the stage's bundle. `+registry [<site>]` lists each
claim's block, holder, term, epoch, declared family, first reading, worked tonnes against the floor
and the contest window, then the paper's rows — the issuer named once in the head, not per row — and
a lapsing claim as "term ended …, lapsing". The lapse's canon line is the clerk's, "{holder}'s claim
on block {L} at {site} lapsed." (`tacsim.claim.lapsed`, naming the holder, guarded by
`records_for_source`), not a draft of the player's (OC10). A refused act's escrow is held up to an
hour before it refunds (OC14). OC1, OC7, OC8, OC10, OC12 and OC14 are the owner-critical defaults
the lead accepted, standing until the owner rules.

### 9.4 Trespass and contest

In phase 1 a licensed rig **will not bite claimed ground**: the mining gate refuses, and the lore is
the Registry interlock (config `[tacsim.surface] registry_interlock = true`), a lever that bounds the
engine — no theft-and-enforcement model until the fight exists. Knowledge is free: a TV may drive onto
and sound claimed ground, and its own way home never depends on tenure (§8). Conflict enters by the
legal path: **contest by discovery priority** — within 30 IC days of registration a rival with a
licence and a hand-made **sounding the site heard** of a field in the block, stamped **inside the
block's current tenure epoch** (after it last opened) and *before* the holder's evidence, may
`+claim/contest`; the stamps decide, the claim transfers, the contester pays the stake and the term
fee on transfer and the loser's term fee is refunded (this needs the seize-transfers-tenure fix,
`provider.rs:2171-2200`). Evidence from an earlier epoch — a sounding made under a previous claim, a
former holder's own — counts for nothing, so a lapse closes the old book. Because soundings are
heard, a staker knows the count before paying (§9.3). Pledge and transfer of claims are the economy's
as built.

**As built (SM.6):** the seize-transfers-tenure fix is built as the `seized` state's optional
`holder` and `discovered` parameters (`IndustryFamilies::transfer_tenure`): the claim goes
`Uncontested` under the new holder, its `discovered_at` the stamp passed or now (a later stamp
refuses, as does a stamp without a holder or a holder with another state), and the epoch advances
once for the block event; without a holder a seizure is exactly as before and the epoch stands. The
fix round (its F1) declared the two parameters on the **staff** offer only, refused for any other
actor, because an issuer's or a jurisdiction's agent could otherwise rewrite the holder with no
title, transferability or pledge check (a lead-level narrowing of the brief, standing until the owner
rules); SM.10's contest carries its challenger as a server-authored row of its own and passes the
heard sounding's real stamp (D10). `@claim/seize` is unchanged.

**As built (SM.10):** the contest is filed from aboard the hull on the site (OC8) as a
server-authored row of its own, `TenureKind::Contest`, carrying the heard sounding's real stamp and
the challenger's fresh term; the seizure's `holder` parameter stays staff-only. Its window runs from
the block's last opening (`opened_ic`, the lapse the stake was made against; none for epoch 0) to
the holder's discovery stamp, and a take-over's or a transfer's own epoch advance never narrows it:
the window opens at the last opening, never at a take-over or a transfer (D18, OC17). The
contester's fee is accepted by the clerk after the transfer lands, and the claim runs a fresh term
from the transfer (`transfer_tenure_termed`); since the lead's fix round
`ClaimRecord::transferred_termed` sets that term in the transfer's own post-image, so a contest
moves the claim's revision by one like every other write. The loser's term fee is refunded by the
clerk as a fresh idempotent post from the Registry's account, never minted, only once the loser's
own fee settled (a second contest waits on it with `CLAIM_PENDING`); the contest reads the cooling
term too. A landed claim whose fee is cancelled, amended back to a draft or re-offered on other
terms is lapsed unpaid by the clerk with cooling (D19, OC16). The build let an expired fee stand and
page (D19's second half, the judge's R1); the lead's fix round superseded that: a fee that expired
unaccepted lapses the claim unpaid **without** cooling (kind `expired` under the same
`tacsim.claim.unpaid/…` receipt, `EXPIRED_LAPSED` told once, industry's `on_lapse` writing no
cooling row when `cooling_until` is not after the lapse, a cross-generation pair read as cooling),
while a fee the Registry accepted that then expired across an outage is forfeit to the Registry and
counts as paid everywhere — the claim stands, the refund pays and a contest's loser hears
`CONTEST_LOST`. A lapse is told once, even after an unreadable claim log (the round). A re-stake
after an expired lapse needs a fresh hand-made reading in the new epoch (§16). "Pledge and transfer
of claims are the economy's as built" does not hold for surface claims, which are `Bound` (§9.2).
The interlock is `[tacsim.surface] registry_interlock`, default `true`, explicit in every shipped
config, and `false` is refused at boot (`TACSIM_INTERLOCK_REFUSED`, OC13); nothing in the game reads
it. OC8, OC13, OC16 as the round amended it and OC17 are the owner-critical defaults the lead
accepted, standing until the owner rules; whether a contest whose fee is withdrawn after the
transfer landed should restore the loser's claim, and whether an expired-unaccepted renewal fee
should lapse the claim or revert its term, are open for the owner (§15).

### 9.5 Zones

Each block carries a zone in lane data: `core` for every phase-1 block. Frontier blocks (attackable
without notice, doc 10 5A) arrive with the fight (§17); permanent NPC extractors fall under the
declared-operation regime as doc 10 5A says.

### 9.6 Occator and the Syndicate

Occator holds Occator Rim's inner blocks: two worked by its drones (§12.3) and the rest reserved for
its phase-2 expansion and operator agreements (a 20 % royalty in kind at the cycle). Its **outer
blocks** are open, authored rich (`prospectivity`), and carry **Pallas Syndicate paper claims**, seeded
`Contested { contract }` with their adjudication contract — extending doc 11 §5's one seeded outer
claim to each outer block — the lore reason the Syndicate filed. A licensee may work or stake one:
two issuers may claim one place and a Contested claim still authorises (`record.rs:864-866`), so both
stand and the player mines. **The Registrar's rule**, deterministic and carried **inside the player's
own first `+claim/renew`** of a papered block, so no Registry actor stands on the path (D-18-4): a
block worked for at least 100 t in the term has the Syndicate paper suspended under the site's issuer
and the player's claim becomes Uncontested; a block worked under 100 t is not renewed and the paper
stands. Worked ground is honoured, with no roll and no Registrar LLM. The authority for it is a
**jurisdiction row on the contest offer**: today `ClaimContest` is offered only to staff or to an
actor who acts for the claim's own issuer (`provider.rs:1539-1555, 2165-2189`); the site's issuer gains
the right to move the contest state of any claim whose subject is a deposit at that site, and nowhere
else, server-authored onto that renewal's offer as the stake is. When doc 11's faction cut lands, a
Syndicate paper refused at Ceres advances a new **Outer Claims** front (proposed here; doc 11 §6 has
none, §18), and registered claim-holders are made Council members in `mush-faction`, which the
`FactionStandingProvider`'s member predicate then reads (`mush-faction/src/standing.rs:33-58`).

**As built (SM.6):** the jurisdiction row is a composed `JurisdictionAuthority` seam whose default,
`ActsForJurisdiction` (`jurisdiction_claims`), adds to the contest offer every claim over a deposit
whose recorded `OreDeposit.jurisdiction` the actor `acts_for`, so the Registry may suspend a foreign
claim at its own sites and nowhere else; the fixed rows are re-checked in `contest()` before anything
is read. It does not by itself carry the Registrar's rule inside the player's own renewal (a player
does not act for the Registry): that is SM.10's own composed authority, or a server-authored row on
its renewal offer (D8). Every surface deposit records its site's issuer, the Registry, as its
jurisdiction at opening, and a site whose issuer has no lane org row opens nothing, fail closed: the
step records the site failed and pages once, and the pad stays closed until the map is repaired,
since a deposit is never re-opened (lead ruling 7, the fix round's F3, the lead's default standing
until the owner rules).

**As built (SM.10):** the paper is lane v12 data — `BlockDef.paper { issuer, contract }` and
`SiteDef.papers { row: { bond_major } }`, Occator Rim's twelve outer blocks (A–E, H, I, L–P) under
the row `occator_outer` with a 5,000 bond — and the Pallas Syndicate is a Ceres lane org (50,000,
faction `syndicate`). The site clerk files the paper's adjudication contract (offered by the tick's
GOD under the Syndicate's root scope, party and adjudicator the Registry, the bond never escrowed, a
century's WorldIc deadline, never accepted or ruled), then seeds one block a fire from lane v12 (the
lead's ruling 10): Occator's four held blocks as its Uncontested claims under the Registry for a
century (OC3), and each papered block's paper on its block record once the contract is logged (OC2).
A player's claim on a papered block is filed Uncontested from the start, "Contested" being the
paper's state (D7, D20, OC5), so "the player's claim becomes Uncontested" yields. The Registrar's
rule rides the player's own renewal as the `Renew` row's `suspend` and `by` lists (the lead's ruling
11) — no Registry actor and no `JurisdictionAuthority` row — suspending the paper's claims and the
block record's paper at 100 t or more worked in the term. OC2, OC3 and OC5 are the owner-critical
defaults the lead accepted, standing until the owner rules.

**As built (SM.12b):** "two worked by its drones (§12.3) and the rest reserved" is lane v15's
`site.yard` on Occator Rim: the Dray-P "Occator Yard" `OWP-YD-01` settled at the pad's arrival pose
(0, 0) with no glyph (O-13), two rigs `OWP-YT-01/02` of Occator's pool in its bay, and a DARK
dispatcher; F and G are worked, and all four held blocks (F, G, J, K) are surveyed at R1 as from a
rock stand (§5's note). Under `KEY_1` F holds 271 t of fair-or-better ice in 22 pockets, G 256 t in
20, J 252 t in 23 and K 252 t in 19 (pinned by T0). The site clerk's order is: the held seeds, then
the house's standing licence (one normal-lane handler, 3 writes; §9.1's note), then one house survey
a fire (`tacsim_dispatch` under SM.6's attempt rule), then the papers; the licence and survey items
answer nothing whenever they cannot progress, so the papers never wait on the house (D26, the lead's
ruling). O-8 and O-9 (the yard's names, its two rigs and its hull; the blocks worked and the survey)
are the owner's provisional rulings of 2026-10-24; O-13 is the lead's default, standing until the
owner rules.

---

## 10. Surfaces — D-20-13

Two **pulled grid feeds** on the `atlas.map` shape, as the chart panel is (`port/chart_panel.rs`), so
the one grid printer and the web's grid decoder read them: **`site.map`** — a drive view of 28 by 28
cells (10 m) around the addressed rig at `cell_columns 2` (56 columns) and a whole-site view of 32 by
32 cells (80 m; a cell shows the best digit of its four pockets) at `cell_columns 1` in the TUI and 2
in line mode, each block's letter in its corner cell: terrain glyphs from the atlas vocabulary with
the three footings and the crater told apart, `H` the hull, `@` your rigs (`1`/`2` beside them when
two are out), `o` other units, digits `1`–`3` for your own R2 readings in the chosen family (`+` where
another family also reads), `.` sounded and barren, `v`/`V` the public pits, `#` a held block's edge;
beside it the vector block with the readings list (`C117 fair metal 120 m NE 25 s`; `C-NW fair–rich
metals (5–16 pockets) 300 m N`), the family shown, each rig's current leg for client interpolation,
hopper, charge and lock, and the hold's unreserved room — and **`body.ground`** (§3.1). A small pushed
`tv.status` keyframe per act change carries the status line, and every completion prints its line
(§1).

**As built (SM.3b):** the pushed `tv.status` keyframe is not built (it is SM.9's); act changes reach
the operator through the completion lines and `+tv`'s status. The leading slot runs `/1` to `/4`
(§4.3).

**As built (SM.4a):** the readings-list example reads `(5–16 pockets)`, corrected in place above
from Draft 0.5's `(5–8 pockets)`, which would exclude the truth whenever the field is rich: the
count is the union of the band's word ranges (the lead confirmed it). The scan's and the sounding's
completion lines (`SCANNED`, `SOUNDED` and their ice and barren forms) print the reading block; a
text beyond the brief's list, `READINGS_DARK` — "Rig {n} finished its {act}; its readings are on
your chart, but the instruments cannot read the ground right now." — is told when the row stands but
the site's ground cannot be read at the tell, and `READINGS_GONE` only for a row gone, unreadable or
under another rule (D26, narrowed in the fixer round).

**As built (SM.9):** the pushed `tv.status` keyframe landed, carrying the operator's whole
projection, not a status line: `{ schema: { id, shape: status, version: 1, retain: "latest" }, rig:
<TvRec> }` to the operator alone, pushed from `write_projection` only when the store wrote, kept in
the session's per-feed slot and never the resume ring, silent in line mode. "Each rig's current leg
for client interpolation" holds as built: the TUI and the web place the viewer's own rigs along
`rigs[].leg` from `site.clock` and `site.under` by SM.4b's `leg_position` rule and re-pull on
`tv.status` only, with no timer (the lead's 0z ruling); the TUI moves a rig on its own paints, about
every 4 s, and the web repaints once a second while a leg is in flight. The projection's log is its
current operator's excursion's (SM.9's D16): a recover's tell starts a clean log, so the log pane
holds the open excursion only and empties when the rig comes home (a residual the owner should see,
§16). `site.map` gains vector keys, no glyph, `print` or `lines` line moved: the legend pairs `v`,
`V` and `#`; `site.addressed`; `site.family_word` and `site.families`; `site.grid { x0, y0, span,
cell_cm }`; `site.marks { letters, hull, slots }`, listed where the composed grid shows that glyph;
`site.under`; `site.clock { now_ic, rate }` only while a rig of the viewer's has a leg on the site;
`site.log` (at most `MAP_LOG_LINES` 6); `readings[].target` and `.line`; `rigs[].line`, `rigs[].run`
(from the projection, the viewer's own only, through `MAP_RUN_CLAUSE` or `MAP_CORE_CLAUSE`) and
`rigs[].queued`; `hold.line`; and, from the lead's fix round, `site.no_readings` and
`site.other_rigs`, each `string | null` — `MAP_NO_READINGS` whole, null with readings in view, and
the other-rigs sentence whole and unwrapped, null with none in view — which `lines` still carries,
wrapped for line mode, and which no client reads out of `lines`. The glyph collisions SM.4b left
are resolved for the screens by `site.marks` (on the TUI the letters dim, the hull bold and the slot
digits reversed; on the web labels, the ship and digits), while line mode keeps the glyphs.

**Line mode** is the floor. `+tv/map [family]` prints the drive view windowed on the rig (the family
with the most readings in view by default, named in the block); `+tv/map site [family]` the whole
site; every grid print carries one legend line (`@ you  H hull  o rig  , regolith  ^ rock  = ice
O crater  1–3 ore (t a cycle)  + other family  . barren  v/V pit  # claim`). `GRID_MAX_COLUMNS` and
`GRID_MAX_ROWS` rise from 78 by 20 to **118 by 32** under the 120-column ruling
(`mush-ansi/src/panel.rs:18-22`; `docs/help/tui.md:40-41` still says "drawn for 80 columns" and is
corrected), so the 32-row site view fits in one print. **The TUI**: one lettered screen, **`q`** (free
in both key tables; `j` is the alternative), `MAX_SCREENS` 11 → 12: Left the rigs' status (32), Main
the map (58), Right the readings and the log (28), 118 at the floor; `z` toggles ground chart and site,
`l` the family layer, `1`/`2` the rig, `s`/`d`/`x`/`m` prefill scan, drive, sound and mine, `r`
re-asks; no mutating verb on a bare key. **The web**: a `FeedScreen` on the `.site` feed, hotkey `q`,
an SVG site picture and a 2:1 body picture, click-to-prefill. One blessed fixture renders
byte-identically on all three.

**As built (SM.4b):** `site.map` landed as a pulled grid feed, registered after `sky.chart`, and
`+tv/map [site] [family]` prints its frame in line mode through the TUI's own row decoder (a new
crate edge, tacsim → mush-ansi), so line mode, the TUI's and the web's decoders read the blessed
fixtures alike; the TUI screen `q` and the web `FeedScreen` are SM.9's. The legend is 122 columns,
past the 118-column cap, so line mode prints it over two lines broken at the last entry separator
(`  # claim` the second), the two joined being the legend verbatim; the frame's `site.legend_line`
holds the whole string (the owner may shorten the legend instead). The readings list holds at most
12 rows, nearest first, and drops the example's trailing drive time ("25 s"), since no rule gives it
without a path a row (P9, an owner-critical default the lead accepted, standing until the owner
rules). The slot digits `1`/`2` share characters with the ore digits, and the block letters `H` and
`O` read as the hull and the crater on the site view: collisions this section chose, left as
amendment candidates for SM.9's screen. The drive view's frame carries `origin_cell` and `extent
[256, 256]`, the site view `origin_cell [0, 0]` and `extent [32, 32]` (80 m cells), and
`units[].cell` and `you` are window-relative frame cells. The hold's unreserved room, `hold.room_t`,
is the `stow.ore` compartments' mass limit less their load less each of the hull's rigs' hopper load;
SM.7 adds outstanding cycles to the same function. The pits `v`/`V` read SM.6's cursor, and `#` is a
stub for SM.10. Line-mode grids rose to 118 by 32 (`GRID_MAX_COLUMNS`, `GRID_MAX_ROWS`),
`docs/help/tui.md` is corrected, and `+tv/map` joined `RUN_ALLOWLIST` in three copies, not two
(`screen.rs`, `run-allowlist.json` and `stores/screens.svelte.ts`). Neither `+tv` nor the map wants
a cold home field — the map wants only a cold site — so SM.9 must not rely on the map to warm one.

**As built (SM.9):** the TUI's screen `q` and the web's quarry `FeedScreen` landed. Screen `q` is
the twelfth (`MAX_SCREENS` 12, the third gated), after the chart and before the cockpit: the rigs
(32), the map (58) and the readings and the log (28), all on `site.map`. `z` steps drive → site →
ground → drive, not a toggle; `l` the family layer; `t` picks the next rig, not `1`/`2`, since the
digits are global screen keys on both clients; `s`/`d`/`x`/`m` prefill and `r` re-asks. The web's
screen sits on the `site.map` feed (the Draft's `.site`), hotkey `q`, with the TUI's eight keys, an
SVG site picture with click-to-prefill and a 2:1 ground picture with a flat toggle. A `site.map`
keyframe that no longer lists the rig `t` picked clears the pick on both clients, a reason frame
included, so the next ask names no rig (the lead's fix round); clearing on any frame that lists no
rigs also drops the pick on a cold or dark frame (accepted), and a cleared pick does not re-ask by
itself (SM.10's row). `PanelSubscribe` carries an optional `params` object (at most 512 bytes, an
object only, else `PANEL_PARAMS_REFUSED` before anything is sent), which the gateway forwards as the
query's args, so the web asks for a view, rig, family or body. A rig in the bay with a queued unload
shows `QUEUED_LINE` under its line on both screens. The rig key `t`, `z`'s cycle, client
interpolation with the TUI's ~4 s step and every new player-facing text (`MAP_RUN_CLAUSE`,
`MAP_CORE_CLAUSE`, the legend labels "pit, worked", "pit, worked out" and "claim", the TUI's and
the web's hints and headings) are the lead's owner-critical defaults, standing until the owner
rules.

**As built (SM.10):** the claims are on the map. `claim_edges` draws every live claim's block
perimeter less its letter corner as `#` (never over a letter), the same for every viewer, so the
SM.4b note's stub is gone; the frame's `site.claims` `[{letter, state: claimed|held, holder,
yours}]` is a vector key, omitted when empty, so no blessed fixture moved, and the clients draw no
more of it than the grid's `#`. The `+tv` ground line reads the record (§1 beat 4's note), and
`+chart`'s `TENURE_LINE`, the `sky.chart` feed, `+chart/map`, `body.ground`'s
`ground.sites[].tenure` and `+chart/ground` count live claims from the records with the word "{n}
claimed" (§1 beat 2's note). SM.9's "a cleared pick does not re-ask by itself" is answered by the
one-shot re-ask SM.10 inherited: a reason frame that clears the pick while the quarry is the screen
is re-asked exactly once on both clients (`quarry_map_ask()`, `refreshQuarrySite()`); a frame that
again lists nothing asks nothing more, a frame that drops the pick but lists rigs asks nothing, and
off the quarry nothing is asked.

**Verbs** (every `+tv/…` act previews bare and commits with `go`; an optional leading slot `1`/`2`
names the rig): `+tv` (status per rig: block and tenure, field, footing, stores, the no-readings hint),
`+tv/lease [take|return]`, `/launch`, `/scan` (alias `/survey`), `/sound [quiet]`, `/drive
<addr>|<x,y>|<block>|<bearing> <m>`, `/mine [family] [cycles]` (alias `/drill`), `/mine stop`,
`/recover`, `/unload [qty]`, `/dump`, `/plan <addr>[,…] [family=<good>]`, `/plan show|clear`, `/map
[site] [family]`, `/forget <site>|<body>`, `/quiet`; `+coop/refit prospector|standard`;
`+chart/ground <body>` and `+chart/ground map <body>`; `+ship/lift <port>`; `+ship/order await tv
[<slot>|all]`; `+licence/buy|show`; `+claim/stake|renew|release|contest|list`; `+registry <site>`.
The cockpit's bare selectors (SCAN, DRIVE, MIND) stay the arena's; `+tv/mine` and `+tv/scan` collide
with nothing (the m/mi/min rule applies to bare selectors only, `match_/cockpit.rs:204-215`). Reads
join the `RUN_ALLOWLIST` (both copies). The help page `docs/help/mining.md` lands its outline — the
twelve beats, their verbs, the `go` rule and the slot rule — with SM.0; `ships.md`, `trade.md`,
`coop.md`, `getting-started.md` and `tui.md` gain lines. (As built: the outline did not land with
SM.0, and `docs/help/mining.md` does not exist at SM.3b; the lead's SM.4a carries the mining help.)

**As built (SM.4a):** `docs/help/mining.md` landed with SM.4a — the twelve beats with their verbs,
the `go` and slot rules, landing at the lettered sites, what a reading says and the heard sounding,
the verbs still to come marked "(not yet)" — and `ships.md` points at `help mining`; SM.8 completes
it. `+tv/quiet` takes `[on|off]`, and `+tv/forget <site>|<body>` previews bare and strikes with
`go`; `RUN_ALLOWLIST` is untouched. `+tv`'s status gives a rig on the site's ground its own line
under its rig line — block and its authored tenure word, field, footing and the reading or the
no-readings hint — with a cold-site and a dark-layer form (§1 beat 4's note).

**As built (SM.4b):** `+tv/<n> drive <target> [go]` takes a pocket, a field, a block letter, a site
cell `x,y` or a bearing `<octant> <metres>` (`ne 300`, `northeast 300 m`); `+tv[/<n>]/map [site]
[family]` is a pure read; and staff's `@geology <site> <pocket>|<field> [province|field|pocket]` names
the site by name, location or entry and reads truth under its block's authored shift. `help mining`
gains the map and its legend, the drive and the recover's drive home.

**As built (SM.7):** `+tv`'s status prints `RUN_LINE` (with `RUN_STOP_CLAUSE`) or `CORE_LINE` under
a drilling rig, the act's line after it; `TV_USAGE` and `TV_HELP` gain `+tv[/<n>]/mine [family]
[cycles] [go]` (alias `drill`) and `+tv[/<n>]/mine stop`, and `help mining` builds beat 8. The map's
`hold.room_t` now counts every outstanding cycle (§6's note); the map's lock and run fields and the
two-family `V` are SM.9's.

**As built (SM.8):** the unload verb is `+tv[/<n>]/unload [<t>] [go]`, or `unload` as a first word,
`<t>` 1 to 99 tonnes and `go` last, else `UNLOAD_USAGE` — not `/unload [qty]`; `TV_USAGE` and
`TV_HELP` gain it, and `+tv`'s status prints `QUEUED_LINE` ("Unloads its hopper into the hold
next.") under a rig with a queued unload. `docs/help/mining.md` is complete — beat 9's recover with
its unload and `+tv/<n> unload`, beat 12's sale, the reserve rule, the pits and the fog, every verb
line — and `ships.md`, `trade.md` ("only the hold sells ore"), `coop.md` and `getting-started.md`
gain their lines. The quarry screen's line for a queued unload is SM.9's.

**As built (SM.9):** the ground chart's verbs are `+chart/ground [<body>]` and `+chart/ground map
[<body>] [<family>]` (`+chart/body` untouched), and `+chart/ground` joined `RUN_ALLOWLIST` in all
three copies. `docs/help/tui.md` gains the lettered screens `c` and `q`, `ships.md` the two
`+chart/ground` lines, and `docs/help/mining.md` beat 2's two verb lines and "The quarry screen";
its "Still to come" no longer lists the ground chart of a whole body (the lead's fix round). The
quarry screen's line for a queued unload landed (above).

**As built (SM.10):** `+claim/stake|renew|release|contest|list` (in the authoritative bracket) and
`+registry [<site>]` (read-only, beside `+credit` and `+chart`) landed; `+registry` and
`+claim/list` joined `RUN_ALLOWLIST` in all three copies; `docs/help/mining.md` builds beat 10, its
"(not yet)" and "Still to come" staking clauses removed. Every new player-facing text — the brief's
texts verbatim, `WHICH_RIG_STAKE`, `CLAIM_WHICH`, `TENURE_SUMMARY`, the lapse's canon line and "{n}
claimed" — is OC10, the lead's owner-critical default, standing until the owner rules.

**As built (SM.11):** the plan's verbs are `+tv[/<n>]/plan <list> [family=<good>] [go]`,
`+tv[/<n>]/plan show` and `+tv[/<n>]/plan clear`, beside the switch form `+tv[/<n>]/plan/show` (the
`show` switch admitted only beside `plan`, D11), which `RUN_ALLOWLIST` names as `+tv/plan/show` in
all three copies; `TV_USAGE` and `TV_HELP` gain the forms, and `+tv`'s status prints the asker's own
plan line under its rig. `+ship/order await tv [<slot>|all] [until=<IC hours>]` files the await, and
the queue reads back through `+ship/orders`. The plan is on the site map as the author's own
`rigs[].plan` vector key — `no`, `state`, `step`, `steps`, `next`, `deadline_ic`, `end` and `text`,
`null` for every other viewer — rendered under the rig by the TUI's rigs pane and the web's quarry
screen. `docs/help/mining.md` builds beat 11, `orders.md` gains the `await tv` vocabulary and
`ships.md` the lift rule.

---

## 11. The numbers, with their basis

| Number | Value | Basis |
|---|---|---|
| Site | 256 × 256 cells at 10 m, 2.56 km; 16 blocks of 640 m; 4 named fields a block; 256 addressed pockets of 40 m a block; the pad at the centre (1.3 km to any pocket along an axis, 1.8 km straight across) | D-16-5's arena window and `MAX_TILE_CELLS`; a block as the tenure unit keeps a site from being three claims |
| Geology | province one chart cell (6.43°); field 320 m (lean 1–4, fair 5–8, rich 9–16 ore pockets, shifted by an authored `prospectivity`); pocket 40 m (lean 3–6, fair 6–12, rich 12–24 t; up to two families) | ≤ 64 ore pockets a block a family by construction, so the cursor sidecar's cap never binds; a site is thousands of tonnes a family against the corridor's 4,000 t season; as built (SM.1): at most two families a pocket, the ranges code constants |
| Instruments | scan 48 cells each axis (nine fields), sound 16 (a field), quiet 8; footing sets sharpness | per-axis squares, so "a whole field from its middle" is exact; as built (SM.4a): reach by the doubled-cell centre rule (exactly nine fields, 64 pockets and 16 from any stand), sight by the atlas line of sight from the 2.5 m mast to a mast's height over the field's centre cell |
| Cycle | 1,800 IC s (75 real s); rock 3,600 | half `PRODUCTION_CADENCE_SECS` 3,600 (`workorder.rs:69`); four to six cycles a hopper keeps a trip under ten real minutes; as built (SM.7): read from the drill row (`mine::cycle_secs`) |
| Yield | lean 1, fair 2, rich 3 t a cycle, integer, never rising; a blind cycle needs 3 t of room | goods are whole tonnes (`goods.json:13-15`); D-E7-6; fitting never tells the class; as built (SM.7): the drill row's yield, the blind room `mine::blind_room_t` derived from its richest (3 t), never a constant |
| Hopper | 12,000 kg / 13,200 L, `stow.ore`; a run fits the lesser of the hopper's room and the hold's unreserved room | seven hoppers and 2 t of an eighth fill a Dray-P's 86 t, and the eighth run stops at 2 t |
| TV speed | tracked 300 cm/s: regolith 2.0 m/s (5 IC s a cell, 21 real s a km), rock 1.0, ice 1.5 | `locomotion.json` tracked row; as built (SM.4b): the unit's `max_speed_cms` 300 and the battery row's 15 cu per nominal cell with the locomotion table's class costs, no constant in the surface crate; a regolith kilometre exactly 500 IC s and 2,250 cu (pinned), rock 1,000 s and 4,500 cu, ice 667 s and 3,000 cu |
| Charge | 100,000 cu; mine 2,000, scan 1,000, sound 2,000, drive 15 cu per cell-cost; bay recharge 50,000 cu per IC h, unloads included; reserved in each act's start | a hold's worth of cycles never drains it; the reserve rule binds long drives and plans and is enforced once; as built (SM.8): `UNLOAD_S` 600 IC s a code constant, the bay regaining 8,333 cu over it (U11), capped at the capacity; the unload reserves nothing and reads no battery |
| Landing | TWR ≥ 1.2 at departure mass and the plan's throttle against `surface_mg`; Dray 206 kN: Ceres 1.79 laden, Luna 0.30, Titan 0.36, Mars 0.13 | `dray.json:83-92`, `passage.rs:76-80`; the ship map's table; as built (SM.2): Ceres 1,785 ‰ at the rail and its band edge laden at exactly 672 ‰ of throttle, the ascent √(g·R) 360 m/s (Ceres) and 254 (Vesta) paid only at a site end, Phobos 1 mg |
| Hop | `base_s` 3,600 IC s through the laden step (≈ 5,300 out, ≈ 7,000 home with a full hold); burn ceil ≥ 1 t; crew-days by the existing rule; landed 1 crew-day a soul an IC day, floored at one leg home | `passage.rs:235-244, 699, 707-722`; `route.rs:837-846`; as built (SM.2): 5,322 IC s out and 6,893 home for §3.3's loads, 7,200 at the rail; the stay charged at lift; a departure for a site holds back the leg home |
| Dray-P | cargo 86 t / 103.2 m³; a two-slot bay (12 t of TVs); 2.3 t of headroom at the maxima (178,680 kg) | `dray.json:84-92`; never re-rated; as built (SM.3a): exactly 2,320 kg of headroom (178,680 of 181,000 with two occupants; 178,580 with one) |
| Rights | commons and unclaimed open ground: licence only; a staked block: the holder's; knowledge, driving and recovery: no tenure at all | §8, §9 |
| Tenure | licence 2,500 / 168 IC d + 3 IC d grace, extendable anywhere; lease 1,400 a term + 5,000 deposit; claim 2,000 + 2,000 a term; 50 t worked a term to renew; ≤ 2 live a account; renew or take over in place; a lapsed holder's cooling term 168 IC d; contest window 30 IC d inside the tenure epoch | the dormancy week, `lane:2171`; the claim index is add-only; as built (SM.5): the grace authorizes, a buy extends from the expiry while in force or in grace, one licence a gateway account, no cap on buying ahead (the lead's defaults); as built (SM.10): the claim fees lane v12 data (`claim_stake_major`, `claim_term_major`: 2,000 + 2,000 a stake, 2,000 a renewal, 4,000 a contest, the loser's 2,000 refunded), each a Public fee contract its pilot escrows for its real hour; the worked floor read from the cursors less the record's `worked_at_term_start`, 100 t on a papered block for the Registrar's rule; the cooling term binding the holder, anyone acting for it and the holder's gateway account, and none after a fee expired unaccepted (the lead's fix round); a seeded claim a century (OC1, OC3, OC15, the lead's defaults) |
| Prices | `metal_ore` 70 (5:2 → 175 < 200), `ice_ore` 35 (5:2 → 87.5 < 100), `carbon_ore` 50, `strategic_ore` 600; references 3500 / 1750 / 2500 / 30000 minor | each under the finished good's ask so buy-and-mill never loses; D-18-24 amended for ore; as built (SM.12a): buy-and-mill never loses stands, the two lane mills at 5 : 2 (175 < 200, 87.5 < 100) |
| Drip | 40 / 40 / 20 / 2 t a good an IC day (960 / 960 / 480 / 48 a real day), standing caps 240 / 240 / 120 / 12 t, within capital | the clerk's 90-day re-post rule; a hold sells in one act; the knob sizes the ore economy; as built (SM.5): a standing intake renewed within one real day of its deadline, at most one banked drip, a lapsed row back at one drip (the lead's default) |
| Mill | 5 : 2 (the Exchange's `metal_ore` into `metals`, Occator's `ice_ore` into `volatiles`); 20 t a batch; 40 t of ore an IC day a mill; at most 2 batches and 2 restocks a mill an IC day, out of 720 Ceres-clerk fires an IC day at rate 24 | as built (SM.12a; lane v14): buy-and-mill never loses; the drip's rate; the ratio and product fixed for the ledger's life; measured writes 5 a batch (6 into a fresh product pool), 2 a restock, 3 a relisting with a counter write (O-1 and O-5, the owner's provisional rulings of 2026-10-24) |
| Dispatch | re-arm 3,600 IC s (lane data, at most 604,800); 2 rigs; hold 86 t; the delivery bound the mill's `per_ic_day_t` (40 t); 2 of Occator Rim's 32 excursion slots | as built (SM.12b; lane v15): the Dray-P's bay; 86 t of ice is 34.4 t of water a hold; measured writes a delivery 5 (6 into a fresh storefront pool) for mined ore and 7/8 for rooted, seeded ore, a survey 1, the licence 3 (O-8 and O-14, the owner's provisional rulings of 2026-10-24) |
| Income | one TV on fair ground 4,900 cr gross an active hour (4,300 after the Trust's share), rich 6,700 (5,900), the hop round trip included; a second TV nearly doubles it; one landed hold a plan | §12 |
| Tick | 1 s; visit 64, work 16; one act an operator a fire; ≤ 8 writes a completion, ≤ 3 for any other act | the journey tick; `pool.rs:497-506`; as built (SM.3b): `LAUNCH_S` and `RECOVER_S` 300 IC s and `DUMP_S` 0 (the lead's defaults), a finish retried 8 times, then paged and backed off 60 fires; the write bounds unmeasured until SM.12; as built (SM.7): a cycle's completion holds ≤ 8 writes by construction only (the record, at most two cursors, the chart; the ring is the tick's own document), unmeasured until SM.12; as built (SM.8): the unload finish measured 9–19 and pinned (10 one good into a fresh hold pool, 17 two goods, 9 an existing pool, 17 with 8 marker-ring evictions, 19 two goods with 3), over the row, nothing trimmed; SM.12's cost lane gates on it; as built (SM.12a): the mill's batch, 5 writes (6 into a fresh product pool), is a pinned overrun of "≤ 3 for any other act" for SM.12c-lane to gate (the restock 2, the relisting 3); as built (SM.12b): the dispatcher's delivery bundle, 5 (6 into a fresh pool) for mined, unrooted ore and 7/8 for rooted, seeded ore, is the second pinned overrun beside it, never trimmed; as built (SM.12c-lane): the write bounds are measured and gated — `tests/cost_lane.rs`'s census builds every mining-layer act kind at its structural worst case against `support/cohort.rs`'s `WRITE_GATES` — so the clauses' "unmeasured until SM.12" no longer holds: completions hold ≤ 8 everywhere but `done.unload`, pinned at SM.8's 19 handed and gated at the constructed structural maximum of 63 (four ores into four fresh hold pools, each push evicting eight; two goods read 33; the fix round's four-goods unload and body-forget variants raised the builder's first 33 and 3 to 63 and 5); `mill.batch` 5 on the shipped lane (6 rig-only), `dispatch.deliver` 6 into a fresh storefront pool, then 5 (7/8 rig-only), `mill.restock` 2, `dispatch.survey` 1, `site.house_licence` 3, `tvpref` 1; the measured overruns of "≤ 3 for any other act" (O-20, the lead's default standing until the owner rules): `forget` 5 (a body's forget, one row per Ceres site), `claim.stake` 9, `contest` 6, `release` 6, `renew` 5, `unpaid` 6, `lapse` 6, `seed` 7, `site.open` 5, `site.worked` 6, `licence.buy` 14, `lease.take` 19 and `lease.return` 15 (normal lane); the surface fire's release p99 0.77 ms at ten TVs (C2's gate, under 5 ms) and 1.4 ms over the 30-TV month (max 9.0 ms) |
| Caps | 32 excursions a site; 1,024 world-wide; 4 an operator (2 a Dray-P); ≤ 1 notice a trip, ≤ 1 sounding line a sounder an IC hour; frames < 256 KiB | the Compact's span of control as an engine bound; as built (SM.3b): counted over open excursions in the start's prepare, across every lessor pool's rigs; as built (SM.4a): the sounding line's budget per (sounder, site), kept across a forget, to connected listeners only (the lead's defaults); as built (SM.4b): a frame's readings list at most 12 rows, a leg at most 32 waypoints (frozen within 80 m), the home cache 32 fields (about 330 KiB each); as built (SM.7): a run at most eight cycles, two lock rows, a mined ring of 64 facts a site; as built (SM.10): the heard count's scan at most 512 chart rows ("at least N" past them, OC12), a claim order's life 3,600 IC s (`CLAIM_ORDER_TTL_IC_SECS`, 150 real s at rate 24), at most 4 claims a tenure act, the claim log's maps at most 16 rows each, two live claims an account counted over the lane's sites × A–P and its young orders; as built (SM.11): offline only trip-level lines are queued (an unload, a refused act, a run ended by a right, a plan's parking or end, an idle return, a stranded rig; §8), the notice ring's cap of 32 standing; a plan at most 16 addresses and 64 steps with 8 notes, a wait re-decided after 300 IC s (`PLAN_WAIT_IC_SECS`), the idle return at 24 IC h (`IDLE_PARK_IC_SECS`), `await tv` at most 288 IC h; as built (SM.12b): the dispatcher shares one submission slot across the port arm and both rigs' plan steps and queued unloads, and its rigs count in the operator, site and world excursion censuses; as built (SM.12c-lane): the content bounds `MAX_PROGRAMME_POOL` 32 and `MAX_VEHICLE_POOL` 64 (O-19, the owner's provisional ruling), the shipped lane at 8 / 8; the economy's `MAX_LIVE_CONTRACTS_PER_PARTY` 32 caps a cohort at 30 TVs (O-19c, open for the owner, §15) |

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
capital raised by tranche as lane data, as built (SM.5) one appended `raises` tranche of 250,000;
the Exchange's own), and leaves it only through the mill
into asks the trade run already sells from. Eight charter hulls at half a leg a day draw about 80 t of
concentrate and 320 t of water; one fair miner's 70 t of ore an hour is 28 t of concentrate, so
**three miner-hours replace the Exchange's daily draw** and the drip caps the rest at 960 t of each
bulk ore a real day. Prices are authored plus the reach premium as today; ore never moves an authored
price. **Occator's drones** are NPC excursions on its two inner blocks under its own licence and
claims, run by a dispatcher principal on the same tick (bounded to four excursions, in-kind supply
through its own mill, never a cash mint), so "the corridor's deposit and extractor drones" of doc 11
§4.1 become supply the ledger can see, and Occator's 9,600 t water seed lasts a season rather than a
month. The Pallas Syndicate mines nothing on the corridor; its paper is pressure.

**As built (SM.10):** the Pallas Syndicate is a Ceres lane org (50,000, faction `syndicate`,
appended last at Ceres), whose paper is a filed, never-ruled adjudication contract over Occator
Rim's twelve outer blocks (§9.6; OC2, the lead's default, standing until the owner rules).

**As built (SM.12a):** "leaves it only through the mill" holds: the mill's batch is the only path
by which ore leaves the finite economy (§7.3's note), and milled ore is consumed with no tailings
(O-3, the lead's default, standing until the owner rules).

**As built (SM.12b): Occator's drones.** The yard is lane v15's `site.yard` (§9.6's note): the
Dray-P "Occator Yard" on Occator Rim's pad, two house rigs in its bay and a DARK dispatcher, whose
own `Arm::Dispatch` runs after the site clerks, one act a fire. It either delivers the yard hold's
ice ore to Occator's storefront for the mill — one receipted inventory `transfer` under the
`tacsim.dispatch` ledger, the ordinal its replay guard, bounded to the mill's one IC day (40 t),
never a market call or an economy post (O-11) — or files one quiet in-bay rig's SM.11 fill plan
through the player's own compiler under the re-arm rule (§8's note). Player ore displaces drone
supply: the storefront holds at most a day of milling, so once players feed the bid the drones
park (O-10). The house's rigs count in the operator, site and world excursion censuses and never
sell: the dispatcher has no helm (§16's note). "Lasts a season rather than a month" is the mill fed
by the drip, from players' ore and the drones': the drones alone give F and G 597 t of ore, 238 t of
water, 0.74 real days of the 320 t draw against a 3.84-day ceiling (all four held blocks 1,148 t,
1.43 days, ceiling 7.68), measured by T14 and never asserted (O-16). O-10 and O-14 are the owner's
provisional rulings of 2026-10-24; O-11 and O-16 are the lead's defaults, standing until the owner
rules.

**Levers, all data**: the ore bids, the drip and its caps, the mill ratios, the hopper, the cycle, the
yield table, the field and pocket distributions, the authored prospectivity and worked shares, the
fees, the worked floor and the cooling term, the lease, the references for the share.

**As built (SM.12a):** "the mill ratios" is a lever only at a mill's first authoring: a mill's
ratio and product are fixed for its ledger's life (O-5, the owner's provisional ruling; D9), while
its `batch_t` and `per_ic_day_t` tune freely; on a world that has milled, a changed ratio or product
stops that mill Flagged until it is authored back. As built (SM.12b): the yard's `rearm_ic_secs`
(lane data, at most 604,800) is a lever too.

---

## 13. Cuts

Each cut is one commit on `main` with the Fable trailer, pushed when green, with a `## SM.n` heading
in **`docs/design/20-cut-notes.md`** (its own file; Q-20-13). Sizes as doc 18 §5: S a day, M a few
days, L a week or more. One lane version per cut, as the A-cuts did. No deploy without the owner.

**As built (SM.1–SM.3b):** SM.1 `61324cba` (with `6f797974`), SM.2 `bd718fb2`, SM.3a `96cc7303` and
SM.3b `eb99c9be` have landed, each pushed with its section in the cut notes. The lead split **SM.3**
into SM.3a, the definitions and assets (the vehicle and its carrier), and SM.3b, the clock (the
excursion family, the tick and the acts); and **SM.4** into SM.4a, knowing (the chart documents,
scan, sound, quiet, forget, the status line, the implied launch, reading rule v1 pinned, the mining
help), and SM.4b, driving and the map. SM.1's off-loop driver and SM.2's `@geology <site> <addr>`
were deferred to "SM.4" before that split; a bare "SM.4" elsewhere in this document means both
halves. The version labels moved: doc 19's cut A.8 took lane v8 and goods v2, so SM.2 is lane
**v9**, SM.3a lane **v10** and goods **v3** (the four ore rows, moved from SM.5 because the hopper and
the Dray-P's cargo need `stow.ore`), SM.3b moved no version, and SM.5 is lane **v11** with no goods
change.

**As built (SM.5, SM.4a):** SM.5 `64cd1f8e` and SM.4a `354832bf` have landed too, each pushed with
its section in the cut notes. SM.5 was built before SM.4a (the lead's ruling: its brief was ready
first and it depends on nothing of SM.4's); SM.6 builds after SM.4a, since its brief leans on
SM.4a's chart module, and SM.7 then needs SM.4b and SM.6. SM.5 is lane v11 as planned; SM.4a moved
no lane, pack or digest.

**As built (SM.6, SM.4b):** SM.6 `70c2cd12` and then SM.4b `d367bf3b` have landed, each pushed with
its section in the cut notes; neither moved a lane, pack or digest (`LANE_VERSION` 11 stands). SM.4b
crossed the lead's 20 ms planning threshold at about 30 ms in release, the cost inside the atlas's
`path`, and the lead accepted it with an atlas cut owed after SM.7 (SM.12's row). **As built
(AP):** that cut landed as AP `74c5801f` (an engine cut in `mush-atlas` with no row of its own; its
cut note is `## AP` in `20-cut-notes.md`), between SM.7 and SM.8; it moved no lane, pack or digest
(`LANE_VERSION` 11 stands).

**As built (SM.7):** SM.7 `dd48ed45` has landed, pushed with its section in the cut notes after the
lead's fix round; it moved no lane, pack or digest (`LANE_VERSION` 11 stands). The debts it hands on
are on SM.8's, SM.9's, SM.10's and SM.12's rows.

**As built (SM.8):** SM.8 `9922c9c7` has landed, pushed with its section in the cut notes after the
lead's fix round; it moved no lane, pack or digest (`LANE_VERSION` 11 stands). The debts it hands on
are on SM.9's, SM.11's and SM.12's rows.

**As built (SM.9):** SM.9 `16b0ca4e` has landed, pushed with its section in the cut notes after the
lead's fix round; it moved no lane, pack or digest (`LANE_VERSION` 11 stands). The debts it hands on
are on SM.10's and SM.12's rows; SM.11 inherits nothing new from it.

**As built (SM.10):** SM.10 `fed09da2` has landed, pushed with its section in the cut notes after
the lead's fix round; it moved lane **v12** (the claim fees beside the licence's, the Pallas
Syndicate org and Occator Rim's paper; digest `9108a455…`) and no goods table. The debts it hands on
are on SM.11's and SM.12's rows.

**As built (SM.11):** SM.11 `09854d6a` has landed, pushed with its section in the cut notes after
the lead's fix round; it moved no lane, pack or digest (`LANE_VERSION` stayed 12; the relay
backbone train took v13 after it, SM.12's row). The debts it hands on are on SM.12's row (the marker
ring's horizon, SM.12c's decision (c), and a plan's drive searching twice); its SM.8 handoff, the
orphaned queue, was built as a wait with a page, not a drop (§16).

**As built (SM.11r, SM.12a, SM.12b, SM.12c-bot):** SM.11r `8cc0aa64` (doc 20a §2's row; no row
here), SM.12a `9df93246`, SM.12b `1b5dd371` (with the lead's follow-up `f171b36c`) and SM.12c-bot
`d2f4ee7c` (with the lead's by-hand cohort run recorded in `d842fdd0`) have landed, each pushed with
its section in the cut notes, SM.12a after the lead's fix round. The SM.12 row was built as the
lead split it, its three parts given rows of their own below; SM.12c-lane, the cost lane, landed
after them (below). The SM.12 row's mill clause is met by the SM.12a row, its NPC clause by the
SM.12b row and its bot clause by the SM.12c-bot row, and its tick p99 and write clauses by the
SM.12c-lane row. SM.12a moved lane
**v14** and SM.12b lane **v15**, no goods table; SM.12c-bot moved no version. Relay R10
(`57c5747d`) made `tacsim.relay.luna_ceres` require the lane at an exact version, so each lane cut
re-signed the relay seed beside it, entries unchanged: v3 for lane 14 (SM.12a, the lead's round),
v4 for lane 15 (SM.12b) (§16; doc 15b). The debts the three hand on are on the SM.12 row, for
SM.12c-lane.

**As built (SM.12c-lane):** SM.12c-lane `389383f1` has landed, the train's last cut, pushed with
its section in the cut notes after its fix round; it moved no lane, pack or digest (`LANE_VERSION`
stays 15). With it the whole of SM.12 has landed — SM.12a `9df93246`, SM.12b `1b5dd371` +
`f171b36c`, SM.12c-bot `d2f4ee7c` and SM.12c-lane `389383f1` — and with SM.12 every row of the
table below has landed: **the surface-mining train is complete as built**. The debts SM.12 handed
it are answered on the SM.12 row; what it hands on (a committed cost-lane baseline and delta step,
the per-party contract cap at cohort scale, the plan path's search reuse, the staff ring-clear verb
and the `cost_lane` binary's 90 s budget) has no row, since no SM cut follows (§15, §16).

| Cut | What | Size | Depends on | Acceptance |
|---|---|---|---|---|
| **SM.0** (**landed**, `11c50e84`, `f3df3205`) | This document; the README row (and the 19 and 20 rows moved into numeric order); `20-cut-notes.md` started; `docs/help/mining.md`'s outline (the twelve beats, their verbs, the `go` and slot rules, landing at the lettered sites). **As built:** `docs/help/mining.md` was not landed and does not exist at SM.3b (SM.3b's cut note); the lead's SM.4a carries the mining help. **As built (SM.4a):** its outline landed with SM.4a (`354832bf`); SM.8 completes it | S | — | every Q-20-n carries a default; the README rows in numeric order |
| **SM.1 the ground** (**landed**, `61324cba` + `6f797974`) | `tacsim::surface::geology` (HMAC over the fourth key family, the province/field/pocket passes with the authored `prospectivity` shift and two-family pockets, the reading rule with a version); the geology key family in `mush-config` and its fingerprint record; `@geology`; the off-loop site composite LRU and geology table; the `i16` validator rule on `datum ± relief` and the symmetric cast. **As built:** the in-RAM LRU of sixteen, the pure builds and a synchronous door landed, and the off-loop driver is SM.4's, its first consumer (the lead's ruling); `@geology` reads the body-fixed lattice (`@geology <body> <x>,<y>` with an optional level), the site-address form with SM.4. **As built (SM.4a):** no act's path builds a site ground — the surface tick warms one wanted site a fire behind a non-building peek, and a true off-loop runner is SM.12's cost-lane call (§3.2) — and the site-address form moves to SM.4b | M | SM.0 | a fixed test key gives pinned digests of a 256 × 256 site; a property test over 10⁶ lattice points shows every field's word is the honest summary of its pockets and ≤ 64 ore pockets a block a family; a wrong fingerprint disables the layer with the reason on the readiness line; `provider_rng_policy` green. **As built:** the 10⁶ lattice points read as ≥ 10⁶ pocket evaluations grouped by field and block (1,000,192), beside the pinned whole-site digest |
| **SM.2 sites and landing** (**landed**, `bd718fb2`) | lane **v9** (doc 19's A.8 took v8): `BodyDef.surface_mg/radius_km/geology` and the Ceres and Vesta geology tables, `LocationDef.lat/lon`, the `site` block (per-block tenure, `prospectivity`, `worked_permille`, zone, issuer) with the zero-orgs rule and the serde defaults, the `freeport_registry` org and treasury, five Ceres sites and Rheasilvia Commons with routes; atlas region packs (recipe, the pad room at the centre with `port.dock`, craters); the sixteen block place Things and the site clerk Thing a site; the landing law at the plan's throttle; the hop leg with the ascent term and the 1 t floor; landed life support with the one-leg-home floor and the stores line; the berth and arrival lines; `+ship/lift` docked or settled; recall counts hops; `+chart` lists sites with their survey words. **As built:** the location role `extraction`; the body tables inline on `BodyDef`; six sites over twelve atlas packs; the stay charged at lift; the leg-home reserve with a reserve-aware throttle hint, `SITE_CLOSED` and `LEAVE_PAD` added; `build_site_ground` refuses a region whose body is not the spec's (SM.1's residual), and the lane's tables author `field.barren = {barren: 1000}` | M | SM.1 | `+ship/plan ahuna` with a full tank, one TV aboard, a 120 crew-day locker and one soul plans ≈ 5,300 IC s and 1 t (as built: 5,322 IC s), frozen in the ticket; a Luna site refuses by TWR at planning; arrival settles on the pad and `+ship/lift ceres` works; a hull landed ten days still lifts; a week of hops trips no recall; the two old routes' pins unmoved; sixteen place objects and a clerk a site; land-and-leave survives a restart |
| **SM.3a the vehicle and its carrier** (**landed**, `96cc7303`; SM.3's first half, split by the lead) | goods **v3** (the four ore rows under `stow.ore`, moved here from SM.5); `UnitClass::Vehicle`; `LogisticsDef.drive` optional; the tools content kind (drill, scanner and a battery row); `tv_prospector.json` and `dray_p.json` (a two-slot bay); lane **v10** (the programme's `refits` row, Occator's rig lessor); lay-up (eight rigs in Occator's shed, each a TV Thing, unit, cab, hopper store and `tacsim.vehicles` vault; a hidden bay on every programme hull); `payload_kg` counts the bay; `+coop/refit` (re-binding the hull's unit in place); `+tv` (status) and `+tv/lease [take|return]` with the engine-owned deposit; a charter's end with rigs aboard (`RECALL_BAY`, `BUYOUT_BAY`, repossession) | L (SM.3's, split) | SM.2 | a Dray-P with two TVs aboard plans the same figures as a Dray at equal mass; a lease refused without a free slot (**as built:** the licence refusal is a seam SM.5 fills; it landed in SM.5, which refuses a lease without a licence in force or in its grace, `LEASE_LICENCE`); `live_endpoints` never lists a hopper for any player |
| **SM.3b the clock** (**landed**, `eb99c9be`; SM.3's second half) | the `tacsim.excursion` state family (one `RigRecord` a TV) and the `tacsim:tv` projection; the `tacsim.surface` tick; every act's authoritative start (`tacsim_surface_act`) with the charge reserved in it, and its finish (`tacsim_surface_done`); the slot selector (`/1`–`/4`); `+tv/launch`, `+tv/recover` (the placement half, from beside the ramp; the drive home is SM.4b's), `+tv/dump` onto each site's spoil vault; `TV_OUT`, `TV_BUSY`, `TV_AWAY` and `HOPPER_LOADED`; the `go` rule; `+tv` authoritative; no lane, pack or digest moved. The implied launch moved to SM.4a | L (SM.3's, split) | SM.3a | launch refused under way and at a port; recover from two cells refused; a restart mid-launch completes at the due instant; a return refused with the TV out; departure refused with a TV out; a bare `+tv/<act>` commits nothing; two acts that each fit the reserve rule alone cannot both start when together they would not (**as built:** through the one act slot and the ordinal; SM.7 proves `reserved_total` in the world; since SM.4a the test runs on scan and sound, the test world's Probe kind retired) |
| **SM.4a knowing** (**landed**, `354832bf`; SM.4's first half, split by the lead) | the chart documents (coverage, `ChartProvenance`, `+tv/forget`); `+tv/scan` (nine fields, sharpness by footing) and `+tv/sound [quiet]` (a field) as timed acts with the site-wide notice and its budget, `+tv/quiet`; the `+tv` status line (block and tenure, field, footing); the implied launch (prepending SM.3b's `launch_plan`); reading rule v1 pinned literally; the mining help (`docs/help/mining.md`); no tenure on any knowledge act. **As built:** the chart the typed family `tacsim.chart`, one record per (character, site), the per-body caps enforced at the start (the lead's ruling; §5); the heard budget per (sounder, site), kept across a forget, to connected listeners only, and the ground line's cold and dark states (the lead's owner-critical defaults); `+tv/quiet` a preference document; the site ground warmed on the surface tick one site a fire, its warm peek a capture plus one equality check (the fix round); the pocket byte's tier mask and reserved pit-seen bit and the stamp-ordered provenance ring (the fix round); `READINGS_DARK` and `CHART_UNREAD` | M (SM.4's, split) | SM.3b | readings equal the quantiser for covered cells only; a second principal sees no ore glyph; a repeat from the same cell is refused byte-identically at no cost; a sounding reaches other parties as a bearing and band and nothing more, a quiet one reaches nobody, a tenth sounding in an IC hour prints no line; a sounding from a field's middle reads all 64 pockets and a scan from a field's middle reads all nine fields; a regolith scan prints bands that contain the truth; a rig without a licence scans and sounds on a claimed block. **As built:** K1–K14, seven more, and the fix round's fallback and guard tests (32 in `tests/knowing.rs`), each through the real verb, land path and tick, with reading rule v1 pinned by `tests/geology.rs::sm4a_reading_rule_v1_is_pinned_literally`; the 1,024-field cap is unreachable on the lane, so the 64-block cap is tested end to end and the field cap by a unit test |
| **SM.4b driving and the map** (**landed**, `d367bf3b`; SM.4's second half) | `+tv/drive` with every target kind (A* over the composite at the site budget, the frozen leg in `TvRec.leg`, IC timing, charge, the reserve rule with a real trip home); the drive home `+tv/recover` lacks; no tenure on any movement act; the `site.map` feed, `+tv/map` with the legend line and block letters, the completion lines; the grid caps raised to 118 by 32; fixtures. **Inherits (SM.4a):** `@geology <site> <addr>` with the typed address parse (SM.1's and SM.2's deferral), the real `home_cu` for the reserve rule, `want_site_ground` at the drive's arrival, and the site map's transient heard mark. **As built:** all four inherited items landed; one A* a command and the leg frozen for display within 80 m (§8; the owner-critical default); the per-site home field, keyless while dark, a recover held only to its own leg and never reading the battery (§4.1; the owner-critical defaults); the far recover as one `Recover` act (§4.3); the legend over two lines and the readings rows without a drive time (§10); `@geology`'s site form; the D14 planning threshold crossed at about 30 ms in release and accepted with an atlas cut owed (SM.12's row) (discharged by AP `74c5801f`: 3.3 ms); a new crate edge tacsim → mush-ansi | M (SM.4's, split) | SM.4a | a rig without a licence drives and recovers on a claimed block; 1 km of regolith arrives at exactly 500 IC s and 2,250 cu; an act that would strand the TV is refused; the arrival line prints over SSH; the observer-rule witness over SSH and web. **As built:** met — the first by D8 (a lapsed licence drives into Occator's held block F and recovers), the second by the unit test and D3, the third by D4, the fourth by the loopback test, the fifth by D12/D13 and the TUI and web fixture tests; the freeze census accepted 64 of 64 field middles on every lane site, all at the 1-cell tolerance, at most 24 waypoints (cap 32), path cells 17–140 |
| **SM.5 legal and market plumbing** (**landed**, `64cd1f8e`) | `with_extraction_licence` wired to `tacsim.licence.freeport`; lane **v11** (SM.2 took v9, SM.3a v10): the Ceres ore bids with their drips and caps and Occator's tranche, the ore `reference_minor` rows; `+licence/buy|show` as the player's act under the server-authored issuer, the fee a public contract accepted anywhere, extension while Active, behind SM.3a's `licence_seam`; the ore goods (**as built:** goods v3's four ore rows under `stow.ore`, which the Dray-P accepts, landed in SM.3a; SM.5 changes no goods); the drip step on the stock arm with its own predicate; the fixed-reference variant with its validator rule; the sell preview's unfilled line; readiness lines. **As built:** the sell preview is the bare `+market/sell <listing>` (two lines) and `=all`, with `listing_line`'s `wanted N ×` pinned as the unfilled intake, not re-worded; Occator's tranche an appended `raises` list (D-18-27 amended for it); the licence's terms a lane `licence` block on `ceres_port`; the entitlement opened Frozen and activated with the fee contract (the fix round); `+tv/lease` reads the licence; the body geology tables installed by the definition applier (SM.2's residual); a Licence readiness line | M | SM.2, SM.3a | a licence makes `extraction_authorized` answer Ok over a staff-seeded claim; the lapse at IC day 172 refuses at the next act; a buy while Active extends the expiry; 10 t of ore staff-placed in a hold sells at the bid with the Trust's row at a quarter of the upper half; a filled intake posts afresh at `min(cap, unfilled + drip)` within the hour and never past capital, and an 86 t hold sells in one act against the cap; a fixed reference for a good some port asks is refused at load; the market-never-mints probe green |
| **SM.6 the industry seams** (**landed**, `70c2cd12`) | `IndustryHandleCell` (read access for composed providers); `with_system_opener` and the site clerk's opening on the port tick (one deposit a fire, receipted, the ore good as commodity, accessibility 100, the one-point curve; the `worked_permille` transfer to the spoil with its cursor rows; the "opening" readiness state); `OreDeposit.external`; the `ExternalDraws` trait and the sweep invariant; `ClaimRenew` in place, the Abandoned take-over, the tenure epoch and the cooling term; the seize-transfers-tenure fix; the contest offer's jurisdiction row; the pocket cursor sidecar as the lock's home. **Inherits (SM.5):** closing industry's own `@extract/start` and `@extract/cycle` for surface blocks also closes the licence's two residues there — a staff-seeded surface claim extracting after a lapse (reviewer F12 (ii)) and a refused buy's Frozen entitlement unfrozen by hand (§9.1). **As built:** no opener grant (the clerk's identity in `SiteRow.clerk` is its authority), the clerk on its own `Arm::Site`, one row an offer; `tacsim.sites` owns the sixteen block vaults and the spoil while the block place Things stay the applying actor's (§3.1, §3.4); only ore-bearing (block, family) rows open, up to 64 a site (the barren-ness bit an owner-critical default); the pad open with the layer dark; a site whose issuer has no lane org row opens nothing (the lead's default); the typed-state delegations both ways on the TacSim roster held to reads by a tripwire; `transfer_many` (SM.3b's F3, closed here); the `TenureRecord` per block place, its epoch advanced once per block event, the cooling rows (§9.2); the seize-with-holder path staff-only (the fix round, §9.4); the `JurisdictionAuthority` seam (§9.6); held vaults listed before machine vaults in staff endpoint offers; F12 (ii) closed by the refusal of a surface block, and with it the Frozen-entitlement residue the row's Inherits names | M | SM.1, SM.5 | two boots with the same key open identical reserves; each block's gross reserve equals the sum of its pockets and the spoil holds the worked share, with the invariant green on the first sweep; the applier submits nothing and a re-apply opens nothing twice; a site opens up to 64 deposits over 16 places (64 on an all-families table; only its ore-bearing rows on the real tables) and never attempts a ninth on one; `@extract/start` refuses a surface block; a forged pool debit quarantines the block; renewing nine times never reaches `MAX_CLAIMS_PER_SUBJECT`; a block staked, lapsed and re-staked ten times still accepts a stake; the lapsed holder is refused for a term and a stranger is not; the site's issuer may suspend a foreign claim at its site and nowhere else; a seizure that names a holder transfers the tenure (staff-only until SM.10); every E7 industry suite green. **As built:** pinned by `tests/deposits.rs` (14), mush-industry's `tests/sm6.rs` (15) and mush-inventory's (4), with every E7 suite green; measured on the shipped tables, Ahuna, Juling, Occator and Rheasilvia open 48 deposits in 49 port fires, Kerwan 64 in 65 and the worked Commons 48 in 97 |
| **SM.7 mining** (**landed**, `dd48ed45`) | `+tv/mine [family] [cycles]` / `+tv/drill` and `stop`; the start's gates — tenure (commons, unclaimed open, own claim), the hopper and the hold's unreserved room, the 3 t blind headroom, the charge reservation — and the preview from the operator's own row (the one-cycle run on an unread pocket); the pocket lock on the sidecar; the completion bundle (transfer of the ore good, cursor, receipt, the charge spent) and the tick's log, chart row and line; the `tacsim.surface.mined` ring; terrain-scaled duration; pits on the map. **Owed first (SM.3b's F3):** a dump or a cycle that opens two or more new pools on one endpoint must land — mush-inventory's `add_pool` writes the endpoint's census once per new pool, so such a bundle always refuses today; the census must be written once a bundle (an inventory-owned `transfer_many` or a mush-core re-stage). **Landed (SM.6):** as mush-inventory's `transfer_many`, which stages the endpoint's census once a bundle; the dump already runs on it. **Inherits (SM.5):** the mine start calls `licence_gate` before `extraction_authorized` and passes the record's `lapses_ic` as a fixed parameter its prepare re-compares with the world's IC now. **Inherits (SM.4a):** the chart's `PIT_SEEN` bit (bit 2 of a pocket's byte) is SM.7's to write; the readers already mask the tier. **Inherits (SM.6):** a pocket of two families has two lock homes, one per deposit's cursor, so SM.7 writes two lock rows (or accepts per-family locks) and says which; a lock row on a pocket with no ore of that family could push a fully drawn 64-row cursor to 65, so SM.7 locks only that family's ore pockets or raises `MAX_CURSOR_ROWS`; `PIT_SEEN` and the pits derive from the cursor; and `+tv/mine [family]` answers a barren and an ore-bearing (block, family) with the same line, since a deposit's existence is a barren-ness bit (§3.4). **Inherits (SM.4b):** the map's pit layer already renders the cursor's `v`/`V`; `geology::family_of_word` accepts `carbon_feedstock` and `strategic_elements` beyond the brief's list; `port::ship::hold_unreserved_kg` is the one function to extend with outstanding cycles. **As built:** every owed and inherited item landed — the census once a bundle (SM.6's `transfer_many`), `licence_gate`'s rule with the frozen `lapses_ic`, `PIT_SEEN` written by the core, one lock row per family with ore and the barren-ness bit never echoed, `hold_unreserved` counting outstanding cycles in every measure; the run in the record's cycle slot on the one start action (`act = mine`, `mine_stop`), each cycle landing at its own due keyed `…/cycle/{k}` (§6, §8); the lock rows plus the other rigs' run records, a stale lock matched by its row (§3.4, §6); the licence judged at the next cycle's due, the tenure and the lease at landing (§8); `RunEnd::Dark` with its own line and a drilling rig busy to the departure gate, the lease return and the repossession (the fix round; §6, §8); `stop` without `go`, no mining under a forfeit or ended lease, the public pit as one preview clause, an authored held block refusing before SM.10 and the blind core's family from the operator's own R1 row (the owner-critical defaults the lead accepted; §5, §6, §8, §9.2) | L | SM.4b, SM.6 | a fair 7 t pocket yields 2, 2, 2, 1 then "worked out"; `initial − pool == Σ drawn` after a 10,000-cycle soak; the hopper never exceeds 12 t and a hull's hoppers never hold more than its hold can take, with two rigs filling one hold; a named run that does not fit refuses and names the count that fits, and the bare form stops at the hopper or the hold's room; the preview on an unread pocket prints no class and commits one cycle, and with 2 t of room refuses the blind cycle whatever the class; `stop` lets the running cycle yield; a replayed completion is a HIT; a second rig on the pocket refuses with the public line; unclaimed open ground mines under a licence and refuses once staked by another; no licence, a lapsed licence and another's block each refuse with their public line while the rig still recovers and unloads; pits render for everyone; the cycle line prints over SSH. **As built:** pinned by `tests/mining.rs` (23 tests after the fix round) and the loopback test `sm7_the_cycle_line_prints_over_ssh`; the "unloads" half of "recovers and unloads" is SM.8's; the 10,000-cycle soak is the arithmetic half (`mine.rs`), the bundle's conservation M2's sweep after a read run, a blind ore core and a blind barren core |
| **SM.8 unload, sell, the story** (**landed**, `9922c9c7`) | the unload half of `+tv/recover` and `+tv/unload [qty]` with `max_more`; the hold plan generalised to any stowage block while `governed_hold` stays programme-hull-only; the story test on `build_engine` at rate 24 (kit up with all three desk verbs, plan, lift, scan, sound, mine open ground to full, recover, lift home, sell; a restart inside); `docs/help/mining.md` complete; the help tripwires. **Inherits (SM.7):** the "unloads" half of SM.7's acceptance ("recovers and unloads") and `HOPPER_LOADED`'s remedy; what a hopper the hold cannot take does — the hold's unreserved room is read by mining and the map only, while market purchases and other hold writers admit against the hold's limit less its load, so cargo bought mid-run can leave a hopper the hold cannot take: the unload must say what happens, or every hold-admitting writer reads `hold_unreserved`. **As built:** every inherited item landed — the unload as `act = unload` on the one start and finish, queued durably on the record (`RigRecord.queued`) by a `Done` recover and started by the tick as the operator, or filed by hand with `how = whole` or `part` and `qty` (§7.1, §8); the finish one inventory-owned `transfer_many_marked` (§7.1); `HOPPER_LOADED`'s unload remedy on a site's pad and `HOPPER_LOADED_AWAY` elsewhere (§4.3); the answer for a hopper the hold cannot take — the unload's own room `hold_unreserved_with`, no other hold writer changed, what fits unloaded and what stays said (§6); the plan generalised as `stowage_plan`/`plan_check` with `PortCapacity::plan` and `governed_hold` unchanged (§7.1); the story in `mush-server/tests/sm_story.rs`, twin arms over one fixed geology key with a `build_engine` restart inside the mining run; `docs/help/mining.md` complete (§10); an unload only on a site's pad, the queued unload not gated on the operator aboard, the 0 s `UNLOADED_NONE` act and a refused finish never overfilling (R5–R8, the owner-critical defaults the lead accepted; §6, §7.1); `UNLOAD_NO_STOCK`, `UNLOADED_NONE_EMPTY` and the " and " join of multi-good lines (the lead's fix round) | M | SM.7 | the story is green on the production composition with one staff act (the rate); unload refuses under way and with the TV out and never leaves ore behind after an honest fill; the hopper is refused at sale by `HOLD_ONLY_SOURCE` and by `may_take`; T-45 and the legacy-handler tripwires green. **As built:** met — pinned by `tests/unloading.rs` (18: U1–U12, U4b, U9b, A3, the `how`/`qty` rule, the finish's refusals and the counted writes), `mush-inventory/tests/sm8.rs` (5) and `mush-server/tests/sm_story.rs` (2: A1, whose straight and restarted arms run in one test, and the check that the story's shared start is never in the past); the story on `build_engine` at rate 24 sells the unloaded hold at the Exchange's bid with the Trust's share and needs no staff act at all (the rate is the boot's `[game].ic_ratio`; its only GOD use the `@tacsim/port status` reads); its bunkers carry the gaps measured over doc 18's planner (§18's Doc 18 item); the help and legacy-handler gates green |
| **SM.9 the quarry screen and the ground chart** (**landed**, `16b0ca4e`) | `body.ground` (`+chart/ground`, `+chart/ground map`; `+chart/body` untouched); the pushed `tv.status` keyframe per act change (deferred from SM.3b); TUI screen `q` (`MAX_SCREENS` 12) with the rig keys; the web `FeedScreen` and pictures; keys; `RUN_ALLOWLIST`; the boot-parity manifest; fixtures. **Inherits (SM.4b):** the `site.map` frame's keys as built (the site view's `extent [32, 32]` with `origin_cell [0, 0]`, read in 80 m cells; window-relative unit cells; §10); the glyph collisions (`H`/`O` block letters against the hull and the crater, slot digits against ore digits) as amendment candidates for the screen; neither `+tv` nor the map wants a cold home field, so the screen must not rely on the map to warm one. **Inherits (SM.7):** the two-family `V` rule (a pocket worked out in one family and untouched in the other reads `V` on every map); the map's lock and run fields; the run's pocket and family read from the record-backed reads (`+tv`, `site.map`), never the projection, whose `RunView` carries no pocket, family, yield or class (the lead's ruling on the fix round's N4). **Inherits (SM.8):** the queued unload's line on the quarry screen (`+tv` prints `QUEUED_LINE` under a rig with a queue). **As built:** the inherited items landed as follows — the `site.map` keys read as built, now with vector keys of their own (§10); the glyph collisions resolved for the screens by `site.marks` while line mode keeps the glyphs (§10); the two-family `V` standing as built, pinned by a unit test with no code change (§3.4); the run on the map as `rigs[].run` and the queued unload as `rigs[].queued`, `QUEUED_LINE` on both screens. The cut note records no disposition of SM.4b's cold-home-field inherit or of SM.7's lock field (the frame carries no lock key of its own). The SM.7 inherit's "never the projection, whose `RunView` carries no pocket, family, yield or class" is reversed under the lead's ruling 4: the screens read the projection's `RunView`, which now carries `addr`, `good`, `stop` and `blind` (the operator's own choices) and never class, yield or tonnes, `RigRecord.run` is read by no frame, and `TvRec::check` bounds `RunView.addr` at `MAX_RUN_ADDR_BYTES` 8 with its own refusal, "a rig projection's run address is at most 8 bytes", apart from the cycle bound's (the lead's fix round; §8). The clients interpolate the viewer's own rigs along `rigs[].leg` from `site.clock` and `site.under`, re-pulling on `tv.status` only, no timer (the lead's 0z ruling; §8, §10); `tv.status` pushes the operator's whole projection (`rig: TvRec`) from `write_projection` only when the store wrote, and the projection's log is its current operator's excursion's (D16); `site.map`'s vector keys (§10), with `site.no_readings` and `site.other_rigs` added and the stale rig pick cleared on both clients when a held frame no longer lists the slot in the lead's fix round; `PanelSubscribe.params` (≤ 512 bytes, an object only); screen `q` the twelfth, `z` stepping drive → site → ground, `t` the next rig (§10); the ground frame's rows south-first (§3.1). The rig key `t`, the two-family `V`, the ground chart's glyphs and shared-cell listing, `z`'s cycle, client interpolation with the TUI's ~4 s step, the line-mode body map at one column a cell and every new player-facing text are the owner-critical defaults the lead accepted (§3.1, §10, §15) | M | SM.4b | one fixture renders byte-identically in line mode, the TUI and the web at 120 columns; `+chart/body ceres` still prints the elements; the wash is identical for two viewers and a best reading appears only to its viewer; screen-count tests updated; no mutating verb on a bare key. **As built:** met — pinned by `tacsim/tests/quarry.rs` (Q1–Q11: the wash cell by cell, two viewers' frames equal and one byte-identical across the other's sounding, `+chart/body` untouched, the blessed ground fixture, the `tv.status` pushes, a second lessee seeing none of the first's log), `mush-net-ssh/tests/quarry_screen.rs` (11: the twelfth screen, no key submitting, the three fixtures byte-identical at 120 columns, the status re-pull with no timed pull), the screen-count tests at 12 and the web's quarry, screen and game tests; Q8's release timing of the ground frame was not run (SM.12c's, on SM.12's row); the tab strip pinned exactly at the 120-column floor beside the 140-column case (the lead's fix round) |
| **SM.10 claims** (**landed**, `fed09da2`) | `+claim/stake|renew|release|contest|list`, `+registry`; Deposit-subject claims over a block's deposits as the player's act under the site's issuer; the hand-made evidence rule, the stake preview's heard-soundings and cooling lines; the 50 t worked floor, lapse to `Abandoned` with its canon draft and epoch advance, the take-over on re-stake, the cooling term; the interlock lever; discovery-priority contest on a heard sounding inside the epoch with the contester's fees; the seeded Occator holdings and Syndicate paper with their adjudication contract; the Registrar's worked-claim rule inside the renew act; the two-claim cap; claims on the site map. **Inherits (SM.5):** the stake and the renewal call `licence_gate` before `extraction_authorized` (a Frozen entitlement with no record, a refused buy's residue, reads unlicensed there). **Inherits (SM.4a):** contest evidence reads the stamp-ordered provenance ring, never the chart's last stands. **Inherits (SM.6):** every tenure event — stake, lapse, take-over — covers the whole block at one term, since the `TenureMark` records only the last event; the cooling term's `acts_for` half; `block_tenure`'s word computed from `claims_over(Deposit{..})` and `IndustryFamilies::tenure(place)`, falling back to the authored word for an unclaimed block; the claim fees (no constant in `surface::tenure`); the contest's challenger as a server-authored row of its own (the seizure's `holder` parameter stays staff-only), passing the heard sounding's real stamp (§9.4); the Registrar's rule carried by its own composed `JurisdictionAuthority` or a server-authored row on the renewal offer (§9.6); the stake answering a barren and an ore-bearing (block, family) with the same line and fee (§3.4). **Inherits (SM.7):** mining reads tenure by block, so a claim is a whole-block claim; seeding Occator's claims replaces R5's stand-in refusal (an authored held block with no live claim) with real claims; the lapse of a term-expired claim, which reads as open ground until then (R20); and the claim fees (§9.2). **Inherits (SM.9):** a one-shot re-ask, on both clients, when the rig pick clears on a reason frame while the quarry screen is active — a stale ask's `MAP_NO_RIG` frame otherwise stays on screen until the next `tv.status` push or `r` — weighed against its hiding the reason a deliberate `t` onto an Away rig earns (a structured reason key on reason frames is a later cut's); and the claims on the site map as `site.map` vector keys the screens read, never a prefix of `site.lines`, as SM.9's `site.no_readings` and `site.other_rigs` are (`claim_edges` is still the stub, `#` is in the legend pairs and no claim mark is in `site.marks`). **As built:** every inherited item landed — `licence_gate` read first by the stake and the renewal; the contest's evidence read from the stamp-ordered provenance ring (the pocket ring only, R3); every tenure event one `industry_claim_tenure` act over the whole block at one term; the cooling term's `acts_for` half, with a gateway-account half beside it (OC15); `block_tenure`'s word read from the block record `tacsim.claims`, not `claims_over` plus `tenure(place)`, falling back to the authored word; the claim fees lane v12 data, no constant in `surface::tenure`; the contest's challenger row `TenureKind::Contest` carrying the heard sounding's real stamp, the seizure's `holder` still staff-only; the Registrar's rule as the renewal's `Renew` row (`suspend`, `by`), not a `JurisdictionAuthority`; the stake answering barren and ore-bearing ground alike, nothing printed reading the deposit count; R5's stand-in replaced by Occator's seeded claims and R20 closed by the clerk's lapse; SM.9's one-shot re-ask on both clients, and `site.claims` a vector key with `claim_edges` drawn (§10). The block record staged in industry's bundle (D17), `Bound` claims and filed Uncontested (D20), the fees as Public contracts the clerk accepts after landing, the clerk's lapses, release with cooling, the take-over, the window at the last opening (D18), the unpaid-fee lapse (D19), the Syndicate and its paper, the interlock refused at boot, and the lead's fix round (an expired-unaccepted fee lapsing without cooling, a fee forfeit to the Registry paid everywhere, a contest moving the revision by one, a lapse told once after a lost claim log) are §9's notes; OC1–OC17 are the owner-critical defaults the lead accepted (§15) | L | SM.7, SM.9 | staking with a gifted, bought or plan-made reading or none is refused and a hand-made core suffices; a second stake on a block held under the site's issuer refuses and a Syndicate paper bars nothing; a self-issued claim from a player org authorises nothing at a Registry site; a claim worked one cycle is not renewed and the block reopens to a stranger at once and to its lapsed holder after a term; a sounding from before the block last opened defeats nothing and one inside the epoch does; a Syndicate block worked 100 t shows the paper suspended after the player's renewal, one worked less is not renewed, and no Registry actor submits anything; the stake reaches the chronicle through the bridge and the lapse through its draft. **As built:** met — pinned by `tacsim/tests/claims.rs` (35 after the lead's fix round, among them C1–C23, C11b, the fixer's six, C20b–C20d and the lost-log test, each through the real verb, the offer route, the land path and the ticks) and `tacsim/tests/lane.rs` (v12's contents, the v11 rebuild, the paper validator's refusals), with mush-industry's `tests/sm10.rs` (16); a gifted or bought reading cannot exist yet (`Reported` is not built) and a plan-made one waits for SM.11's `via: Plan`; the lapse reaches the chronicle as the clerk's canon line, not a draft (§9.3) |
| **SM.11 plans and offline** (**landed**, `09854d6a`) | `+tv/plan <addr>[,…] [family=<good>]` with the block rule and the frozen family, `show`, `clear` (the compiled vocabulary, one landed hold by the unreserved-room rule, 288 IC h, knowledge-gated, no re-arm, rights lapse skipping to park); `await tv [<slot>|all]` to 288 IC h; the `depart` refusal while any rig is out; idle return-and-park; the notice budget; span of control 4; `via: Plan` on plan-made readings; the offline story across a restart. **Inherits (SM.4a):** heard lines are never queued, so reconnect notices are this cut's. **Inherits (SM.4b):** a recover's `LegShort` must read as "parked where it stands" for idle return-and-park; a `DriveOrder` is RAM, so a restart loses an unlanded one ("not planned; preview it again"). **Inherits (SM.8):** `MarkersFull` at an unload's finish is a wait with the record's back-off, never a park, and is named in the reconnect summary — the hopper vault's marker ring keeps 128 markers younger than a real day (about 64 two-ore unloads; at rate 24 a plan unloading one ore more often than every four to five IC hours, or two ores more often than every nine, can fill it) and is the vault's, so it carries across lessees — and this cut's builder measures the ring's headroom at the story's cadence; if the cap binds, the lead's ruling is to make the ring's window the act's retry horizon rather than a real day; a queued unload whose operator lost the helm or the charter is refused at every attempt until any start lands on the rig, so the plan chain, which reuses `Queued`, drops an orphaned queue with a page. **Inherits (SM.10):** `evidence_at`'s `first_local_at` clause trusts a ring with no `Plan` entry, so it is revisited once `via: Plan` stamps exist (a plan-made reading never qualifies a stake), while `heard_count` and `heard_before` read the pocket ring only; and the plan on the map as a `rigs[].plan` vector key (the lead's ruling at the SM.10 fold). **As built:** every inherited item landed or was answered — the notice budget, offline only trip-level lines queued, hand-made or plan-made (D13; §8); the idle return's `LegShort` read as parked where it stands (`TvState::Parked`; §8); a plan's drive bound by the start's `plan` and `step`, never a RAM `DriveOrder`, so a restart loses nothing of a plan (only a hand filing's RAM order, which previews again); `MarkersFull` a `Markers` wait, never a park, named in the reconnect summary by `WHY_MARKERS_QUEUED` and, since the lead's fix round, retried by the tick, so the unload and the plan go on by themselves when the ring frees (§16); the ring's headroom measured at about 8 markers a landed hold (one every 4.3 IC h in the story) against 128 a real day, the cap not binding, so the ring's window as the retry horizon stays SM.12c's decision (c) (SM.12's row); the orphaned-queue handoff built as the plan's own helm wait with a page, not a drop, since no principal can land one (the lead's ruling; §16); `evidence_at`'s `first_local_at` clause replaced by `first_hand_at` with the Plan-first trim, and `heard_before` reading hand soundings only (§5); and the plan on the map as the author's own `rigs[].plan` vector key on both screens (§10). The plan as a durable record stepped through the existing acts, its three compiled targets and no `scan` step, `await tv` per rig, the departure held by a live plan in the bay, the bound ending a plan from any wait, the clear's fixed ordinal and the author's show and clear from anywhere aboard (the last three the lead's fix round's) are §8's notes; D2, D4–D8, D13 and the rest of §15's list are the owner-critical defaults the lead accepted | M | SM.8, SM.10 | a logged-off player's plan fills the hold and parks with every hopper empty, the queue lifts, flies and sells, the notices read on reconnect; two rigs' plans fill one hold and neither strands ore; a plan whose licence lapses recovers, unloads and parks with its reason; the plan does not re-arm; killing the server midway changes nothing in the final post-image; a fifth excursion for one operator refuses; a plan-made reading does not qualify a stake. **As built:** met — pinned by `tacsim/tests/plans.rs` (26 after the lead's fix round: P1–P13, P3's three worlds and P5's `plan_changed` world among them, with the round's marker-ring, clear, bound, off-helm and idle-page tests), `tests/orders.rs`'s `sm11_await_tv_files_renders_and_runs`, `tests/excursions.rs`'s `sm11_an_offline_launch_line_is_not_queued` with its positive control, and the plan line on both clients' quarry screens; P1's offline story on Ahuna Flank worked 24 t in 13 cycles over 3 trips in 46,297 IC s (about 12 IC h) and ended `HoldFull`, its arm rebooted twice reaching the straight arm's post-image; the plan-made stake is refused in P9 |
| **SM.12 the mill, the drones, the cost lane** (split by the lead, the whole **landed**: SM.12a `9df93246`, SM.12b `1b5dd371` + `f171b36c`, SM.12c-bot `d2f4ee7c` and SM.12c-lane `389383f1`, rows below) | the Exchange's and Occator's mill arms with receipts and `restock_ask`; Occator's dispatcher excursions on Occator Rim; a harness cohort (64 TVs over 8 sites, every act cycling; tick p99, writes an act, composite churn; the Vesta passage included) beside the arena baseline; a `mush-bot` cohort playing the whole loop over SSH. **Inherits (SM.4a):** a true off-loop keyed site-ground runner behind `Effect::ModuleAsync`, decided on the cost lane's measurements (§3.2). **Owed first (SM.4b):** an atlas cut, after SM.7 and before this cut's cost lane, hoisting a per-class cost and slope table out of `mush_atlas::physics::path`'s inner loop (and, if the lead prefers, a per-(rig, target, ground) memo so `go` after a preview does not search again), since a drive's planning measured about 30 ms in release on the worst case against the lead's 20 ms threshold (§8); SM.4b's planner measurement is informational until it lands. **Landed (AP, `74c5801f`):** the dense row and the heuristic (§8's note); the measurement is a release gate, this cut's baseline 3.3 / 7.0 / 8.3 ms on the build box; the memo was not built (it would be a third kind of entry in SM.7's one-slot `ActOrders`), to be reopened here on the cost lane's measurements if they ask. **Inherits (SM.7):** the write-count measurement — a cycle's completion holds ≤ 8 writes by construction only (§11). **Inherits (SM.8):** the unload finish's writes, measured and pinned at 9 to 19 against §11's ≤ 8 (the brief forbade trimming the inventory's writes), which this cut's cost lane gates on; a staff tool to clear a hopper vault's marker ring, or a per-lease ring, if wanted. **Inherits (SM.9), SM.12c's:** Q8's release timing of the `body.ground` frame — the Ceres frame with Ahuna's rows measured in debug at least 20.59 ms, median 21.98 ms, the release run not made — on the cost lane that measures the feeds. **Inherits (SM.10), SM.12c's:** the claims suite's cost — about 65 s on CI's critical path (three-world boots in C2, C16 and C23; 100 t of real cycles in C14a), deterministic and with no wall clock. **Inherits (SM.11), SM.12c's:** the marker ring's horizon — the story's cadence (about 8 markers a landed hold) never binds the 128-a-real-day ring, and about sixteen sold holds in one real day would — the lead's recorded ruling, the ring's window as the act's retry horizon, being the cost lane's decision (c), built only if the cohort's figures ask; and a plan's drive running its A* at the tick and again at the offer, beside AP's unbuilt memo. **Split (the lead's ruling):** this row is built as three cuts — SM.12a the mill (the mill arms with their receipts and `restock_ask`), SM.12b Occator's drones (the dispatcher excursions on Occator Rim) and SM.12c the cost lane (the harness and bot cohorts and the measurements above) — in the order SM.10 → SM.11 → SM.12a → SM.12b → SM.12c. **As built (the split):** SM.12a landed as `9df93246` with lane v14 and SM.12b as `1b5dd371` (with the lead's follow-up `f171b36c`) with lane v15, `PortDef.site` now boxed; SM.12c was built as doc 20a §2 planned, SM.12c-bot first (`d2f4ee7c`, the bot cohort, no version) and then **SM.12c-lane** (the fire's clock, `tests/cost_lane.rs`, C1–C7, the 64-TV month, `scripts/cost-lane.sh` and its `.gitignore` line, decisions (a)–(c) and the owner's table), which landed as `389383f1` with no version: this row's mill clause is met by the SM.12a row, its NPC clause by the SM.12b row, its bot clause by the SM.12c-bot row and its tick p99 and write clauses by the SM.12c-lane row (below). **Plan of record:** the row's implementation plan of record is [20a-sm12-implementation-plan.md](20a-sm12-implementation-plan.md) (Draft 0.4), and since the relay backbone train took lane v13, SM.12a takes lane v14 and SM.12b v15. **As built (SM.12a, SM.12b):** they did, with one addition: relay R10 (`57c5747d`) made `tacsim.relay.luna_ceres` require the lane at an exact version, so every SM lane cut re-signs the relay seed beside its lane (lane v14 ↔ relay v3 at SM.12a, v15 ↔ v4 at SM.12b; §16). **Inherits (SM.12a), SM.12c-lane's:** the mill's batch, 5 writes (6 into a fresh product pool), a pinned overrun of §11's "≤ 3 for any other act" to gate. **Inherits (SM.12b), SM.12c-lane's:** the dispatcher's delivery bundle, 5 writes (6 into a fresh pool) for mined ore and 7/8 for rooted ore, the second pinned overrun to gate; `dispatch.deliver` and `dispatch.survey` are their own act kinds, the filing SM.11's `start.plan_file`; the dispatcher has one submission slot, shared by the port arm and both rigs' surface steps; T5's marker rings stood at 20 and 18 after 6.67 IC days, a projected 128 entries in about 43 IC days. **Inherits (SM.12c-bot), SM.12c-lane's:** its C5 pins name `mining_cohort.rs`'s `the_mining_root_runs_under_the_bots_serde` (every build) and `four_bots_mine_and_sell_over_ssh_in_line_mode` (`#[ignore]`d), and SM.12c-lane's own `ci.yml` comment or stanza keeps the harness parity cut intact. **As built (SM.12c-lane):** the inherits are answered — the SM.4a off-loop runner (decision (a)) and AP's memo (decision (b)) not needed at this cohort, on the figures (§3.2, §8); the SM.7, SM.8, SM.12a and SM.12b write counts measured and gated by the census (§11's Tick row); the SM.8 inherit: the ring tool not built (O-22), the relief instead a per-entry horizon of one real hour on the unload's hopper markers (`UNLOAD_MARKER_KEEP_S` 3,600); the SM.9 inherit: Q8 in release, least 2.61 ms, median 2.65 ms; the SM.10 inherit closed "measured, not reduced" (the claims suite 76.39 s at the cut's final gate); the SM.11 inherit: decision (c) built, triggered by the pre-(c) smoke's mirror projection of about 4,060 against the 3,900 margin, and the double A* measured and not built (§8); the SM.12c-bot inherit: the harness parity pin (`the_weekly_script_runs_the_ci_jobs_two_lines_verbatim`) unmoved and green, the new `ci.yml` comment block sitting after SM.12c-bot's and before `# 16b cut H`, outside the harness parity cut, with no job running it (O-28 (i)) | L | SM.11 | over a simulated IC month the Exchange's concentrate ask rises only by milled ore with conservation asserted; NPC excursions never exceed their caps nor sell into player intakes; tick p99 < 5 ms a fire, ≤ 8 writes a completion and ≤ 3 for any other act; the bot cohort completes the loop in line mode (doc 14 P10). **As built:** the mill clause met by the SM.12a row, the NPC clause by the SM.12b row and the bot clause by the SM.12c-bot row; tick p99 < 5 ms met (release C2's minimum p99 about 0.77 ms at ten TVs; the 30-TV month's p99 about 1.4 ms); the write clause met under the pinned ceilings (§11's Tick row); the harness cohort ran over six sites, not "8" (O-30), and at 30 TVs, not 64, because mush-economy's `MAX_LIVE_CONTRACTS_PER_PARTY` (32) stops a 64-TV world at its kit-up (O-19c, the owner's open fork, §15); the Vesta hulls refused at the epoch (O-26); "beside the arena baseline" met by order, through `scripts/cost-lane.sh` after `harness-weekly.sh`, the committed baseline and delta deferred (D-H-8) |
| **SM.12a the mill** (**landed**, `9df93246`; SM.12's first part, split by the lead) | lane **v14**'s `mill` rows on the Exchange's and Occator's vendor entries and their validator; `port::mill` (the pure batch arithmetic, `MillId`, the `MillLedger` family `tacsim.port_mill`, the `MillProvider` batch bundle, `mill_step` on the stock arm after `drip_step` and before `bid_step`); the market's `restock_ask`, `listing_lapsed` and `draw_reading`; the per-mill `mill_listed` counter on `VendorRow`; the drift check and the Listings line netting milled-not-yet-listed units; the ungated `Binding::Mill` line; one `help trade` sentence (O-2). **As built:** the batch and the restock are two writes, the restock waiting while a Draw is in flight, a refused list step passing its ask over for the IC day (§7.3's note); milled product credited with no tranche or lineage (O-7b); the relay seed `luna_ceres` re-signed at v3 for lane 14, `relay_world`'s `full_star` reading any version from 2 and `relay_content`'s preflight refusing below 2 (the lead's fix round, the judge's path (a); §16); the builder's held lane rows, help sentence and readiness pin applied and the mill tests moved onto the shipped lane (the round); O-1, O-5 and O-7b the owner's provisional rulings, O-2, O-2b, O-3, O-4, O-6 and O-7 the lead's defaults (§15) | M (SM.12's, split) | SM.5, SM.8; built after SM.11 by the lead's order | over a simulated IC month the Exchange's concentrate ask rises only by milled ore with conservation asserted (`sm12a_a_month_of_milling_conserves_and_restocks_only_by_the_mill`). **As built:** met — pinned by `tacsim/tests/mill.rs` (25 after the lead's round, on the production roster: the month at rate 24 over 1,000 fires with one reboot between a batch and its restock, the Exchange milling 1,020 t into 408 t and Occator 980 t into 392 t; test 15's measured writes 5/6/2/3; the round's Draw-wait test), the `mill.rs` unit tests (the pure month 848/840/336/8, `GOLDEN_MILL`), mush-market's ic7 (five `restock_ask` tests) and e7r2's `e7r2_p1_the_restock_creates_no_matter`, and mush-server's `sm12a_a_kept_v13_star_takes_lane_v14_and_relay_v3` (renamed by SM.12b to `sm12b_a_kept_v13_star_takes_lane_v15_and_relay_v4`); `--test mill` about 49 s in debug against its 90 s budget |
| **SM.12b Occator's drones** (**landed**, `1b5dd371`, with the lead's follow-up `f171b36c`; SM.12's second part) | lane **v15**'s `yard` on Occator Rim's site entry (`YardDef`, `YardRow`, `HouseLicence`); the yard hull (a Dray-P settled on the pad), two rigs of Occator's pool in its bay and a DARK dispatcher, laid up as the applier's part (i); the house's standing licence and R1 survey seeded by the site clerk after SM.10's held seeds; the dispatcher's `Arm::Dispatch` (deliver in kind, or file one idle rig's SM.11 plan through the player's own compiler); the re-arm rule; the house view of a lease on every operator, hull-rig and lessee seam; the `tacsim.dispatch` ledger; the ungated `Binding::Dispatch` line; `tests/drones.rs`. **As built:** as §3.4's, §5's, §8's, §9.1's, §9.6's and §12.3's notes say; the relay seed re-signed at v4 for lane 15; SM.11's residual R3 closed (any parking reason kept at the bound, §8); `PortDef.site` boxed; the lead's follow-up made order 5d's remedy read `+tv/{n} unload go`, fixed `TvLease::HOUSE_LEASE`'s doc, read `plan::count`'s test slot before the yard scan and matched T5's and T15's heard-line checks on "rig" as a word; O-8, O-9, O-10 and O-14 the owner's provisional rulings, the rest of §15's SM.12b list the lead's defaults | L (SM.12's, split) | SM.10, SM.11, SM.12a | NPC excursions never exceed their caps nor sell into player intakes; the restart changes nothing; the season measured, never asserted. **As built:** met — pinned by `tacsim/tests/drones.rs` (22: T0–T16 with the house-standing and kept-world apply tests; T5 over 4,000 fires, 22 deliveries of 320 t, 12 hold-fills, 16 loud soundings heard 4 times in all and at most once an IC hour, 742 landings with every ore bid unmoved, 669 house landings with no money and no contract; T14's season 0.74 real days against a 3.84-day ceiling), three yard tests in `tests/vehicles.rs` and the unit tests; `--test drones` 181–190 s in debug, over its 90 s target, reported and nothing shortened |
| **SM.12c-bot the bot cohort** (**landed**, `d2f4ee7c`; SM.12c's bot half, carved out by the lead; the lead's by-hand cohort run on it recorded in `d842fdd0`) | `mush-bot --mine <site>`: a headless, model-free mining lane playing the SM.8 story's loop in a closed vocabulary from its own line session's text alone, with the SM.10 stake and one added loud sounding (`mine::{lines, lane, report}`); the mining rig on `boot_all_in_one`; the every-build canary and the `#[ignore]`d four-bot SSH cohort in `tests/mining_cohort.rs`; the `ci.yml` comment block naming the cohort's by-hand command (O-28 default (i)); `docs/bot.md`'s "Mining pilot". **As built:** the lane retries a closed site's plan and lift, a stale quote and a stale sale, and the world's transient refusals; `DRIVE_HERE` counts as an arrived drive; the block letters A–P are grid glyphs; `--mine` requires `--headless` (the builder's departures, corrections of the brief's model of the game, no engine change); no tacsim, mush-server, mushd or mush-config file touched and no lane version | M (SM.12c's bot half) | nothing of SM.12 (SM.8's vocabulary at HEAD past `7c886408`) | the bot cohort completes the loop in line mode (doc 14 P10; the SM.12 row's bot clause). **As built:** met by `four_bots_mine_and_sell_over_ssh_in_line_mode`, `#[ignore]`d, run by hand and named in a `ci.yml` comment (O-28 (i), the owner's provisional ruling): four real `mush-bot --mine` binaries over SSH in line mode against `boot_all_in_one` at rate 24, two at Ahuna Flank and two at Kerwan Floor, each mining 12 t of metal ore and selling it for 735.00cr net, the stakes F, K, K and J with the second bot at each site refused once, and the heard path proven at both sites with the overlap stamped. The figures are the lead's run on the landed commit (the judge's first residual: the builder's passing run 2 predated the fixer's round): 2,258 s, the world ready after 491 s, per-bot loops 14.7–25.0 min, first launch to last sale 29.4 min, 37.6 min in all. The canary `the_mining_root_runs_under_the_bots_serde` runs in every build, about 500–530 s (about 9 min on the rest shard's `cargo test -p mush-bot` step); `mush-bot --lib mine` five tests, not four |
| **SM.12c-lane the cost lane** (**landed**, `389383f1`; SM.12c's lane half, the train's last cut) | the fire's own clock in the surface tick (`SurfaceFireStats`' `elapsed_us`, `warmed`, `warm_us`, `searches`, `search_us`, `peeks`, `peek_us` and `markers_held`; `Warmed`; `surface_fires`) and the port tick's by arm (`PortFireStats.elapsed_us`, `ArmFireStats` for the stock, site and dispatch arms), all RAM only; the site-ground LRU's counters (`SiteCacheCounters`); the content bounds `MAX_PROGRAMME_POOL` 32 and `MAX_VEHICLE_POOL` 64 (O-19); decision (c), the per-entry ring horizon (`RingEntry.keep_s`, one eviction rule shared by `MarkerRing::push` and `admits`, `transfer_many_marked_keeping`, the unload's hopper markers kept `UNLOAD_MARKER_KEEP_S` 3,600 real seconds, O-22); `tests/support/cohort.rs` and `tests/cost_lane.rs` (C1's four census tests, the coverage test, C2's release gate, C3's restart twin, C4's eighteen-ground LRU, C5's CI-comment and C7's lane-diff pins, the ring pin, (c)'s acceptance, and C6, the `#[ignore]`d month writing `cost-lane.json`); T9's pinned points `[397, 441]`; `scripts/cost-lane.sh`, the `ci.yml` comment block (O-28 (i)), `cost_lane` on shard 2 and `.gitignore`'s `/cost-lane*.json`. **As built:** decision (c) built on the figures and (a) and (b) not needed (§3.2, §8); the 64-TV month STOPPED at the kit-up on the economy's per-party contract cap and ran at 30 TVs, the largest the cap admits (O-19c), the script's default 30 where the test's stays 64 (DEP-C); at 30 TVs no Ceres site holds two hand pilots, so the month's contest check passes vacuously while the census gates `claim.contest` (DEP-B); the month's pilots park when every Ceres site comes home dry for them (the fix round's answer to the review's F5); the plan path's double search, the staff ring-clear verb, a committed baseline and delta step and splitting the binary for its 90 s budget not built; no lane version (`LANE_VERSION` 15, every digest unmoved) | L (SM.12c's lane half) | SM.11, SM.12a, SM.12b; built after SM.12c-bot | tick p99 < 5 ms a fire, ≤ 8 writes a completion and ≤ 3 for any other act, or the pinned measured ceiling where a cut handed one (the SM.12 row's tick p99 and write clauses); the harness cohort over an IC month beside the arena baseline. **As built:** met — the census gates every mining-layer act kind under `WRITE_GATES` (§11's Tick row); C2 in release, five runs of 1,800 one-second fires at ten TVs, p99s 805, 806, 769, 842 and 816 µs, the gate's minimum 769 µs; the month, release under `TACSIM_ASSERT=1`, 30 TVs over 30 IC days (the fixer's final run): surface fire p50 / p99 / max 89 / 1,435 / 9,043 µs (n 108,000), the five windows' p99s 1,388, 1,831, 1,489, 1,092 and 1,099 µs, the port fire 4,713 / 8,552 / 27,490 µs, the other ticks p50 1,227 and p99 3,351 µs, the mirror's max 1,117 (O-27's STOP at 3,900), the rings' least headroom 121, `PlanWait::Markers`, `unload_markers`, `MarkersFull` and back-pressure all 0, 25,739 t mined, loops 17 / 30 / 40, no breach, 449.75 s; C3 a restart inside the cohort changing nothing; C4's LRU counters 54 / 38 / 16 (builds, evictions, entries); the debug `cost_lane` 12 passed and 1 ignored in 134.81 s (105–162 s at default threads, 175 s at two, over the brief's soft 90 s, reported); the six sites (O-30) and the Vesta hulls refused at the epoch (O-26); "beside the arena baseline" met by order — the month is run by `scripts/cost-lane.sh` after `harness-weekly.sh`, never beside it — the committed baseline and delta deferred (D-H-8) |

**Phase 2** (own cuts, after the owner's rulings): the second jurisdiction (Phobos under a Republic
lands office at Stickney with its own licence instrument and a Stickney ore bid; a new `microgravity`
region tag the locomotion table prices); survey trade and commissions (chart rows as
`Representation::Entitlement` listings through a `DeliveryProvider`; Occator survey commissions settled
on `tacsim.survey.logged`); implicit landing anywhere on the lattice; the crewed cab if Q-20-4 goes
that way; org-held claims (`+claim/stake as=<org>`) when players can hold orgs; Occator's expansion
and operator agreements. **As built (SM.12b):** `yard::operator_principal` (`Organization(house)`
for a dispatcher) is the seam org-held claims build on.

**As built (survey commissions):** `+survey [<id>]`, `+survey/accept <id> [go]` and
`+survey/cancel <id> [go]` expose four globally finite Occator offers: `ahuna-a`, `kerwan-f`,
`juling-k` and `occator-p`, for those sites' blocks A, F, K and P. Each pays 500 cr for fresh R2
coverage of all 256 pockets within seven IC days of acceptance; the contract allows one further IC
day for settlement, never for new work. One unfinished commission per gateway account; each offer
pays once globally. Bare acceptance and cancellation preview, and `go` commits.

Acceptance records the exact attempt in the authoritative `tacsim.survey` family before the port
tick has Occator accept and escrow its reward. Successful Sound/QuietSound acts begun strictly
after the acceptance instant add their covered pockets to funded work in the same transaction as
the chart write; Hand and Plan both count. Existing chart coverage, scans, cores and another
operator's readings do not. Coverage and the completion receipt survive chart forgetting and reboot.
The scoped `tacsim.survey.logged` and `tacsim.survey.cancelled` references name the exact offer,
contract and revision and check the immutable consent before settlement; neither discloses a chart.
Payment is automatic. Authoritative cancellation stops progress and selects the original contract's
refund outcome. Cancellation or expiry refunds Occator, and an unfinished offer reopens only after
that refund settles. Completed entries remain closed. Chart trade, its Entitlement delivery and
Reported provenance remain unbuilt.

**Phase 3** (§17): asteroid-field drones; frontier blocks and the fight.

---

## 14. Decisions taken here

| # | Decision | Where |
|---|---|---|
| D-20-1 | The surface loop runs world-side on the IC clock, every act with an IC due instant applied by the 1 s `tacsim.surface` pull tick; never the arena, never an atlas window, never a per-excursion schedule for players. The arena hosts fights, not chores | §2 |
| D-20-2 | A site is a vendor-less port entry with a `site` block over a 256 × 256 atlas region at 10 m (the arena window), 16 blocks of 640 m with per-block tenure, authored prospectivity and worked share, a place object each, four named fields and 256 addressed pockets a block, the pad at the centre; phase 1 is five Ceres sites and a Vesta commons with its own geology. **As built (SM.2, SM.3b):** the site location's role is `extraction`; Occator Rim at 25.4 N; the block places are laid DARK by the applier, owned by the applying actor until SM.6 makes `tacsim.sites` their industry owner, beside a site clerk Thing and a spoil Thing and vault. **As built (SM.6):** the block place Things stay the applying actor's; `tacsim.sites` owns the sixteen block vaults (`Container`-backed by the block places, recorded in `SiteRow.vaults`) and the spoil vault, the deposits' endpoints | §3.1 |
| D-20-3 | The landing law: TWR ≥ 1.2 at departure mass and the plan's throttle against `BodyDef.surface_mg`; a hop's `base_s` 3,600 IC s through the laden step with an ascent term ceiled to a tonne; crew-days by the existing rule and an IC day landed, floored at one leg home and printed. **As built (SM.2):** only a leg with a site end meets the law and pays the ascent; the stay is charged at lift; a departure for a site holds back the leg home, with a reserve-aware throttle hint; `SITE_CLOSED` and `LEAVE_PAD` — the stay charged at lift, the leg-home reserve and `LEAVE_PAD` being the lead's defaults (SM.2's decisions 4, 7 and 10), standing until the owner rules | §3.3 |
| D-20-4 | Geology is a pure HMAC-keyed function under a fourth durable key family with a fingerprint pinned in the world; province, field, pocket in one word set; the four raw families; readings nest; no RNG. **As built (SM.1):** one HMAC per (level, cell, family) under `rustmush.geology.v1`; the fingerprint pinned in `tacsim:geology` at the first keyed boot, a missing or mismatched key a disabled layer on the readiness line, never a refused boot; an authored shift never lifts a province-barren family (the lead's ruling, standing until the owner rules). **As built (SM.5, SM.4a):** the body tables install at the definition applier; no act builds a site ground, the surface tick warming one site a fire. **As built (SM.12c-lane):** the site ground stays built on the tick, measured — the off-loop runner (the cost lane's decision (a)) not needed at this cohort, the month's site warm max 4.5 ms and C4's release build max 4.0 ms against the 20 ms line — and the LRU counts its builds and evictions | §3.2 |
| D-20-5 | One conserved `OreDeposit` pool per (block, family) in the family's ore good, opened at the gross reserve by a site clerk on the port tick through a composed system opener (never the applier), an authored worked share transferred to the spoil with its cursor rows; a bounded pocket cursor sidecar that is the lock's home; every cycle a pool transfer; the invariant through a composed `ExternalDraws` trait on the reconcile sweep; `OreDeposit.external`; a pocket's class fixed, its remainder shown by preview and pits. **As built (SM.6):** no opener grant, the clerk's identity in `SiteRow.clerk` its authority, on its own port-tick arm, one row an offer; a deposit only per ore-bearing (block, family), up to 64 a site, its existence a barren-ness bit SM.7 and SM.10 must not echo; the worked share ⌊t × ‰ / 1000⌋ a pocket, summed and moved by one pool transfer; the pad open with the layer dark, and nothing opened for an issuer with no lane org row; a mismatch quarantines every deposit of the place and provider; the typed-state delegations held to reads by a tripwire (the owner-critical defaults and the lead's, standing until the owner rules). **As built (SM.7):** every cycle one `InventoryFamilies::transfer` from the block vault to the hopper with the cursor's `drawn_t` in the same bundle; `ExternalDraws` green on M2's sweep | §3.4 |
| D-20-6 | The TV is a full `UnitSpec` of a new unfieldable `Vehicle` class with stowage-only logistics and a two-room interior, teleoperated in phase 1; tools are a content kind with per-axis radii; charge bounds a run, reserved in each act's authoritative start under one reserve rule. **As built (SM.3a, SM.3b):** tools are typed documents pinned by digest, with a battery row; the TV is fielded but has no burn row; the control lease is read as the live lessee aboard the settled hull (the lead's default, standing until the owner rules); a zero spend always passes the reserve rule; **as built (SM.4a):** an implied launch's charge reads the bay only to `started_ic + LAUNCH_S`, and a reading on the ground settles without its battery row (the fix round); **as built (SM.4b):** the reserve rule's trip home is real (the per-site home field, keyless while dark), a recover never held to the floor and never reading the battery (P6, an owner-critical default the lead accepted, standing until the owner rules), one with no readable battery row spending nothing for its leg (the builder's decision 3, a default the lead accepted and the owner should see beside P6) | §4.1 |
| D-20-7 | The Dray-P carries up to two TVs in a bay counted in `payload_kg` under the 181 t rail, never a re-rate, each named by its slot; `+coop/refit` swaps it; `+tv/lease` is the player's act under Occator as lessor; `TV_OUT` and `HOPPER_LOADED` refuse departure as backstops; a hopper vault is the system principal's. **As built (SM.3a, SM.3b):** the refit re-binds the hull's unit in place and every programme hull carries a hidden bay; the lease is signed at the Exchange Hall and re-debited at the next `+tv/lease` act; slots `/1`–`/4`; `TV_BUSY` and `TV_AWAY` beside `TV_OUT`; a charter's end with rigs aboard refuses or repossesses and holds the hull while a rig is out (the lead's defaults, standing until the owner rules); **as built (SM.5):** the lease reads the licence (`LEASE_LICENCE`) before the term settle (the lead's default); **as built (SM.8):** the departure's two hopper lines — `HOPPER_LOADED` naming `+tv/{n} unload` first and the dump beside it on a site's pad, `HOPPER_LOADED_AWAY` ("staff must empty it") anywhere else, since an unload works only on a site's pad (R5, the lead's owner-critical default, standing until the owner rules); a queued unload is not busy to the gate | §4.2–4.3 |
| D-20-8 | Knowledge is coverage plus its own provenance enum per principal, re-derived at render; R0 public, R1 the nine-field scan sharpened by footing, R2 the field sounding heard site-wide with a quiet form; only a hand-made reading counts as discovery and only a heard sounding inside the tenure epoch as contest evidence; knowledge needs no tenure; fog lines byte-identical. **As built (SM.1):** the reading rule is versioned (v1) and the regolith band's side is a keyed draw. **As built (SM.4a):** one chart record per (character, site), the per-body caps enforced at the start (the lead's ruling); reach by the doubled-cell rule, sight by the atlas line of sight; a banded field row prints the union of its counts; the heard budget per (sounder, site), kept across a forget, to connected listeners only (the lead's defaults, standing until the owner rules); the repeat rule per instrument class, cell and site | §5 |
| D-20-9 | Every `+tv/…` act previews bare and commits with `go`; `+tv/mine` previews from the operator's own row (one blind cycle on an unread pocket, needing 3 t of room), names its family, commits a run bounded by the pocket, the hopper and the hold's unreserved room across every rig and outstanding cycle of the hull, refuses a named count that does not fit and names the one that does, never clamps; yield by pocket class, never rising; a pocket lock; knowledge acts legal during a run; no `until=full` across pockets for a seated pilot. **As built (SM.7):** the run in the record's cycle slot on the one start action, each cycle keyed per cycle; the lock per family row plus the run-record check; the blind room derived from the drill row; `stop` without `go`, the public pit as one clause, the blind family from the operator's own R1 row, and the lease bar on mining (the lead's owner-critical defaults, standing until the owner rules); a stale lock matched by its row, a drilling rig busy to the departure gate, the lease return and the repossession, and `mine::fits` on `capacity::fits_in` (the fix round) | §6 |
| D-20-10 | Grade is spent at the drill as tonnes; mined matter is raw ore goods under `stow.ore` bought only at Ceres; the hopper never sells; the capacity plan, not the sale predicate, is generalised; the mill is P5's supply half; intakes refresh by a drip step of their own to a standing cap; the Co-op's reference for ore is a fixed minor-unit price at half the bid, amending D-18-24 for goods no port asks. **As built (SM.3a):** the four ore goods landed as goods v3. **As built (SM.5):** lane v11 carries the four Ceres bids and their drips; the drip renews before the TTL, banks one drip and brings a lapsed row back at one drip (the lead's default); Occator's capital rises by an appended raise (D-18-27 amended for it); `reference_minor` is a second programme map. **As built (SM.8):** the capacity plan over any stowage block (`stowage_plan`/`plan_check`), the hopper refused at sale twice and never planned by the market (`PortCapacity::plan` and `governed_hold` unchanged). **As built (SM.12a):** raw ore goods are milled at Ceres by the two lane mills (lane v14) into the house's asks, in kind — a batch's debit and credit under its `MillLedger`, then a `restock_ask` of the units milled — never a cash or a market mint; milled product enters with no tranche or lot lineage, and a mill's ratio and product are fixed for its ledger's life (O-7b and O-5, the owner's provisional rulings of 2026-10-24) | §7 |
| D-20-11 | Every act starts as an authoritative submission writing the excursion's state family and completes on the tick, the normal-lane document a projection; rights are per act — licence and tenure gate mining, staking and renewal only; a fill plan is one landed hold by the unreserved-room rule, 288 IC hours, a closed knowledge-gated vocabulary compiled from a pocket and block list with a frozen family, one a TV, never re-armed, parking when its rights lapse; `await tv [<slot>|all]`; four excursions an operator, two a Dray-P; idle return after 24 IC h; Q-18-22 extended and T13 read as owned-asset production and the character's own instruments on a dispatched sortie, plan-made readings never discovery. **As built (SM.3b):** the state family is one `RigRecord` a TV; the launch and the dump start authoritatively too; every start is `tacsim_surface_act` and every finish `tacsim_surface_done`, mining joining as `act = mine`; "the control lease" is read as the lessee aboard (the lead's defaults, standing until the owner rules). **As built (SM.4a):** the scan and the sounding are acts on the same two actions, a finish off its stand refused `Moved`. **As built (SM.4b):** a drive is another kind on the same two actions (`act = drive`), its leg in the act's params, projected into `tacsim:tv:<tv>` at the first fire after its start and placed once at the due instant; a far recover carries its leg in the one `Recover` act; the start's `to` parameter binds a drive to its RAM order; the leg is frozen for display within 80 m, time and charge the exact path's (the owner-critical default the lead accepted). **As built (SM.7):** the run in the record's cycle slot (`RigRecord.run`) beside the act slot, each cycle finishing at its own due under `…/cycle/{k}`; rights judged at the next cycle's due (the licence) and at landing (tenure, lease); since the fix round a run projected from the first fire after its start, and a dark registry read ending it as `RunEnd::Dark` with its own line, never the claim line. **As built (SM.8):** the unload as `act = unload` on the one start and finish, queued durably on the record (`RigRecord.queued`) by a `Done` recover and started by the tick as the operator; the start's `how`/`qty` bind it to its source as `to` binds a drive; the queued unload not gated on the operator being aboard and a hold with no room a 0 s act told once (R6 and R7, the lead's owner-critical defaults, standing until the owner rules). **As built (SM.9):** the projection's `RunView` names the pocket address and the good and never class, yield or tonnes (the lead's ruling 4, reversing SM.7's); the projection is its current operator's, its log the open excursion's (D16); it is pushed as `tv.status` only when the store wrote; and the clients interpolate a rig's position along its leg (the lead's 0z ruling). **As built (SM.11):** the fill plan is a durable record on the rig (`RigRecord.plan`), its three compiled targets — a loud sounding of a field, a bare mine run of a pocket, every fair-or-better pocket of a block — stepped by the tick through the existing acts as the operator, the drive, the recover, the queued unload, the park and the idle return decided at each step's start and never compiled, so a restart re-decides from the record; one landed hold across every rig's outstanding steps; the 288 IC h bound stopping new steps and never one in flight, and since the lead's fix round ending a plan from any wait; the rights re-read at every step's start, any lapse parking the plan with its reason; never re-armed; `await tv` read per rig (`rig_parked`); four excursions an operator pinned (P8); the idle return after 24 IC h a code constant, `IDLE_PARK_IC_SECS`, the recover submitted as the operator with `by: idle`, never the queue's bound; every plan-made reading `via: Plan`, never a stake's evidence (§5) — D2, D4–D8, D13 and the rest of §15's SM.11 list are the lead's owner-critical defaults, standing until the owner rules. **As built (SM.12b):** "never re-armed" holds for players, while the NPC house is re-armed: Occator's dispatcher re-files a rig's plan after `HoldFull` or `Bound` once the lane's 3,600 IC s have passed, after `WorkedOut` only with fair ore left on its own row and a cycle landed, never after `Cleared`, a right, `Stranded`, `ChartFull` or `Dark` (O-14, the owner's provisional ruling of 2026-10-24); the bound's "ending a plan from any wait" gains that a plan already parking keeps its parking reason when the bound ends it, players' plans too, and only a Live plan ends `Bound` (SM.11's residual R3, closed); the dispatcher mines under the house's fee-less century licence, read by its `licence_standing` (O-12, the lead's default). **As built (SM.12c-lane):** the surface and port ticks time themselves in RAM, and the surface fire's release p99 is 0.77 ms at ten TVs and 1.4 ms over the 30-TV month; AP's memo (decision (b)) not needed at this cohort, and a plan's drive searching at the tick and again at the offer measured and not built (§8) | §8 |
| D-20-12 | One licence instrument per issuer, bought and extended anywhere as a public contract; unclaimed open ground is workable under a licence until staked (changed on the owner's review from "stakeable" alone); blocks as Deposit-subject claims filed by the player's own act under a server-authored issuer; renew in place and take over an Abandoned record on re-stake, advancing the block's tenure epoch; 50 t worked a term to renew; a lapsed holder's cooling term; two live a account; contest by a heard sounding's priority inside the epoch with the contester's fees; jurisdiction is the site's issuer, on the mining gate, the staking refusal and the contest offer, carried inside the player's own acts; the interlock in phase 1. **As built (SM.5):** the licence's terms a lane block; the grace authorizes and a buy extends from the expiry in force or in grace, one a gateway account, no cap ahead, the fee forfeit to the Registry (the lead's defaults, standing until the owner rules); the entitlement opens Frozen and activates with the fee contract, and every gated act reads `licence_gate` first. **As built (SM.6):** the industry seams — `ClaimRenew` in place, the Abandoned take-over, one `TenureRecord` a block place whose epoch a lapse, a take-over or a transferring seizure advances once per block event, at most four cooling rows one a holder, the seize-transfers-tenure fix on the staff offer only (the fix round), and the contest offer's jurisdiction row as a composed `JurisdictionAuthority` — with the player's acts, the cooling term's `acts_for` half and the Registrar's carriage SM.10's. **As built (SM.10):** stake, renew, release and contest are each ONE `industry_claim_tenure` act over the whole block under the site's issuer through the composed `SurfaceTenure` authority; the block record `tacsim.claims` is the one public source of tenure, staged in industry's bundle (D17, OC11); the contest window opens at the block's last opening, never at a take-over or a transfer (D18, OC17); a fee withdrawn after landing lapses the claim with cooling (D19, OC16), the lead's fix round superseding D19's "an expired one stands": a fee that expired unaccepted lapses it without cooling, and a fee forfeit to the Registry counts as paid; claims are `Bound` and a papered block's claim is filed Uncontested (D20, OC4, OC5); the fees lane v12 data, the Syndicate's never-ruled paper and Occator's century claims seeded by the clerk, the Registrar's rule as the renewal's `Renew` row, the cooling term over the holder's gateway account and the interlock's `false` refused at boot — OC1–OC17, the lead's owner-critical defaults, standing until the owner rules. **As built (SM.12b):** the house's licence is a fee-less century licence, an Active entitlement account of Occator at its root scope, opened (or adopted) by the site clerk and recorded as a `HouseLicence` on the site row, not a gateway-account record; the dispatcher's `licence_standing` reads it, its Suspended branch exactly the industry predicate's `licence_accounts` selection (O-12, the lead's default, standing until the owner rules) | §9 |
| D-20-13 | Two pulled grid feeds on the `atlas.map` shape (`site.map`, `body.ground`; `+chart/body` untouched); `+tv/…` verbs with `scan` and `mine` as the words, `go` as the commit and a slot number as the rig; screen `q`; line-mode grids 118 by 32 with a legend line; every completion a printed line. **As built (SM.3b):** a slot switch `/1`–`/4` names the rig; the `tv.status` keyframe is SM.9's. **As built (SM.4a):** `+tv/quiet` a preference, `+tv/forget` its own action; the mining help's outline landed. **As built (SM.4b):** `site.map` landed as a pulled grid feed with `+tv/map [site] [family]`; line mode 118 by 32 landed; the legend prints over two lines and the readings rows carry no drive time (the owner-critical default the lead accepted). **As built (SM.9):** `body.ground` landed as a pulled grid feed at `cell_columns 1`, its rows south-first like every grid frame, with `+chart/ground [<body>]` and `+chart/ground map [<body>] [<family>]`; `tv.status` landed as a pushed keyframe carrying the operator's whole projection (`rig: TvRec`), retain latest, pushed only when the store wrote; screen `q` is the twelfth screen, `MAX_SCREENS` 12, with the rig key `t` (the digits are global screen keys) and `z` stepping drive → site → ground; the clients interpolate the viewer's own rigs along `rigs[].leg` (the lead's 0z ruling); `PanelSubscribe.params` lets the web ask for a view, rig, family or body; the rig key, `z`'s cycle, client interpolation, the ground chart's glyphs and every new player-facing text are the lead's owner-critical defaults (the full list in §15), standing until the owner rules | §10 |
| D-20-14 | Phase 1 is Ceres and a Vesta commons, every block core (amending doc 11 §5's margins), no surface combat; D-16-3 stands; promotion is phase 3 behind an IC-crossing ruling. **As built (SM.2):** the six sites at their authored coordinates; only Occator Rim's outer twelve blocks carry a shift (the lead's defaults, standing until the owner rules) | §3.1, §17 |

---

## 15. Questions for the owner

**Q-20-1 through Q-20-13: every default below was accepted as proposed by the owner on 2026-10-19.**
One default moved afterwards on the owner's own review (§19): Q-20-2's `open` block is now workable
under a licence until staked, not only stakeable (D-20-12); the owner may put it back by a word.

**As built (SM.1–SM.3b, SM.5, SM.4a, SM.6, SM.4b, SM.7, AP, SM.8, SM.9, SM.10, SM.11, SM.12a,
SM.12b, SM.12c-bot, SM.12c-lane):** where this document was silent the cuts took further
defaults — the lead's rulings and the owner-critical defaults listed under each cut's "Deferred,
and why" in
[20-cut-notes.md](20-cut-notes.md) — and each stands as the lead's default until the owner rules;
the "As built" notes name them where they land.

**As built (AP, SM.8):** AP's one owner-critical default is that the debug planning timings never
gate the push-triggered CI shard (§8). SM.8's, standing until the owner rules: R5, an unload only on
a site's pad (so a port departure with a loaded hopper reads "staff must empty it"); R6, the queued
unload not gated on the operator being aboard; R7, a queued unload the hold cannot take at all a 0 s
act told once; R8, a refused finish never overfilling (§6, §7.1). SM.8's new player-facing texts
await the owner's review — `RECOVER_THEN`, `RECOVER_THEN_PART`, `RECOVER_THEN_NONE`,
`UNLOAD_PREVIEW`, `UNLOAD_STAYS`, `UNLOAD_FILED`, `UNLOADED`, `UNLOADED_NONE`,
`RECOVERED_UNLOADING`, `UNLOAD_UNDER_WAY`, `UNLOAD_NOT_SITE`, `UNLOAD_OUT`, `UNLOAD_NO_ROOM`,
`UNLOAD_NO_FIT`, `UNLOAD_CHANGED`, `UNLOAD_USAGE`, `WHY_LEFT_BAY`, `WHY_HOLD_REFUSED`,
`WHY_MARKERS_FULL` (the fix round's), `QUEUED_LINE`, the reworded `HOPPER_LOADED`,
`HOPPER_LOADED_AWAY`, the `+tv` and recover help lines, the act offer's presentation text
("…bring it in, unload its hopper into the hold, or empty it onto the spoil."), and from the
lead's fix round `UNLOADED_NONE_EMPTY` and `UNLOAD_NO_STOCK` — beside the `{ore}` word (SM.7's
`ore_words`, not the label) and the multi-good lines, which the lead's round joined with " and "
where the build joined them with commas.

**As built (SM.9):** SM.9's owner-critical defaults, standing until the owner rules: the rig key `t`
(the digits are global screen keys); the two-family `V`; the ground chart's glyphs `.` `-` `+` `*`,
the marker letters and the shared-cell "Also at F: …" listing; `z` cycling drive → site → ground;
client interpolation, with the TUI's ~4 s step; the line-mode body map at one column a cell; and
every new player-facing text — the `GROUND_*` texts, `GROUND_SITE_LINE_UNPLANNED`,
`MAP_RUN_CLAUSE`, `MAP_CORE_CLAUSE`, the legend labels, the TUI's and the web's hints and headings
(§3.1, §10). **Open for the owner: the passage home from a site's pad.** From a site's pad
`+chart/ground` reads "no passage from <site>" for every other site and never shows the passage
home on the port line, because site ports route only to the Freeport: a routing and text decision
the owner holds — show the Freeport leg on the port line, or route site to site. Nothing is wrong in
the data; the line is what the routes say (§1 beat 2, §3.1).

**As built (SM.10):** SM.10's owner-critical defaults, standing until the owner rules: OC1 the fee
arithmetic (stake 4,000 = 2,000 + 2,000; renewal 2,000; contest 4,000; refund 2,000); OC2 the Pallas
Syndicate (a Ceres lane org, 50,000, faction `syndicate`) and its paper's adjudication contract
(offered by the tick's GOD under the Syndicate's scope, adjudicated by the Registry, a 5,000 bond
never escrowed, a century's deadline, never accepted or ruled); OC3 seeded claims run a century; OC4
surface claims `Bound`, no title; OC5 a papered block's player claim filed Uncontested; OC6 release
a lapse by choice with cooling; OC7 the declared family an optional word, public on the registry;
OC8 the stake from the ground only, the contest from aboard the hull on the site; OC9 renewal at any
time while live, from the term end; OC10 every new player-facing text (the brief's texts verbatim,
`WHICH_RIG_STAKE`, `CLAIM_WHICH`, `TENURE_SUMMARY`, the canon line "{holder}'s claim on block {L} at
{site} lapsed." naming the holder, and the tenure word "{n} claimed", the fixer's reading of lead
note 0a); OC11 tacsim's record staged inside industry's bundle; OC12 the heard count's 512-row bound
and "at least"; OC13 the interlock's `false` refused at boot; OC14 a refused act's escrow held up to
an hour; OC15 the cooling term binding the holder, anyone who acts for it and the holder's whole
gateway account (staff refused too while a row is live); OC16 a fee cancelled, amended or re-offered
after landing lapsing the claim with cooling — the lead's fix round replaced its "an expired one
stands and pages": a fee that expired unaccepted lapses the claim unpaid without cooling
(`EXPIRED_LAPSED`), and a fee forfeit to the Registry after an outage counts as paid; OC17 the
contest window opening at the block's last opening, never at a take-over or a transfer (§9). **Open
for the owner:** (a) whether a contest whose fee is withdrawn after the transfer landed should
restore the loser's claim — as built the block lapses, the contester cools, the loser is told
`CONTEST_LOST` and refunded, and the block reopens; (b) whether an expired-unaccepted **renewal**
fee should lapse the whole claim — as built it does, without cooling, though the earlier fee had
paid for the rest of the term — or revert the term to the paid term's end and leave the claim
standing (a small claims follow-up pins the renewal case either way); (c) the exact per-block heard
log, D12's alternative, against the built 512-row scan.

**As built (SM.11):** SM.11's owner-critical defaults, standing until the owner rules: D2 no `scan`
step (§8 names one; a nameable address is already R1 or finer, and a sounding reads finer); D4 a
plan's steps and the idle recover read the pilot aboard, so a plan waits while its pilot is ashore;
D5 a plan's soundings are loud; D6 `+tv/plan clear` commits with no `go`; D7 the 288 h deadline
stops new steps, never one in flight (since the lead's fix round it also ends a plan from any wait);
D8 a live plan holds its rig from every hand start and holds the hull's departure even with the rig
in the bay, beyond §8's "refused while any TV of the hull is out"; D13 the offline notice budget
applies to hand play too, so SM.3b's `LAUNCHED` is no longer queued; `await tv` read per rig (every
rig parked or its plan ended stranded), the stricter reading of §8's "every TV of the hull is parked
or every plan has ended"; the caps, 16 addresses and 64 steps; a lapsed right parking the plan at
any step's start, a sounding's and a drive's too; every new player-facing text (the `PLAN_*`,
`STATE_*`, `MARK_*`, `END_*`, `WAIT_*` and `PASS_*` lines, `IDLE_HOME`, `IDLE_THEN`, `STRANDED`,
`PLAN_REMEDY`, `PLAN_DOING`, the `AWAIT_*` lines, `count_words`' "1 trip" and "1 cycle", and from
the lead's fix round `WHY_MARKERS_QUEUED`, `UNLOAD_MARKERS` and the `unload_markers` code's "try
again later" for a start nobody types); and the round's `PlanWait::Markers` retry (§16).

**The owner's provisional rulings of 2026-10-24.** Before SM.12a resumed the owner took O-1, O-5 and
O-7b at their defaults provisionally: *"there's too much economy stuff unknown as of yet to pick
numbers, so let's just put in some reasonable defaults and figure it out as we go"* (doc 20a §8).
The same day, before SM.12b and SM.12c built, the owner ruled O-8, O-9, O-10 and O-14 (*"keep the
defaults … I'll tune later"*, the SM.12b cut note) and O-19 and O-25 to O-28 with them (the
SM.12c-bot cut note keeps O-28's default (i) provisionally). These are **the owner's provisional
rulings, not the lead's defaults**: each stands as ruled until the owner tunes it, at the cost of
reversal doc 20a §8 names (a lane-data change after its cut has landed rides the next lane version
or a lane cut of its own; O-7b, the expensive one, rewrites the batch bundle and the never-mints
probes). O-19 and O-25 to O-28 are SM.12c-lane's, built as ruled (below); O-28 default (i) was
built first by SM.12c-bot (below). **Continued 2026-10-26** (doc 20b OD-3): the owner keeps every
provisional ruling as it stands; economy balancing waits until the major gameplay systems are
built, and no economy number is tuned before then.

**As built (SM.12a):** O-1 (5 : 2, 20 t a batch, 40 t of ore an IC day, both mills; its consequence,
that the drip and the capital, not the mill, size the intakes, §7.3's note), O-5 (a mill's ratio and
product fixed for its ledger's life) and O-7b (milled product credited with no tranche or lineage,
doc 18's D-18-26 amended) are the owner's provisional rulings above. SM.12a's lead defaults,
standing until the owner rules: O-2 the `help trade` sentence; O-2b the staff texts verbatim (the
Mill line's templates, the Listings clauses ", {u} t milled and not yet listed" and "its mill ledger
cannot be read.", the validator's sentences, `restock_ask`'s refusals, the mill's warnings), plus
`MillWarn::Storefront`'s new text ("tacsim: a mill's storefront ore cannot be read; the clerk tries
that mill's batch again next fire"); O-3 milled ore consumed, no tailings; O-4 a restock stales open
quotes (`STALE_QUOTE`); O-6 an unreadable ledger flags the gated Listings line; O-7 removing a mill
row pages drift until `@market/vendor withdraw`, with its re-add rule (`docs/ops.md`). The lead's
rulings in the build: the restock waits while a Draw is in flight; the list step's per-IC-day
back-off on a refused ask (`list_refused`); `room_today` reads a later-dated tally as today's; and
the relay seed re-signed with the lane (§16).

**As built (SM.12b):** O-8 (the Occator Yard, `OWP-YD-01`, `OWP-YT-01/02`, two rigs, the Dray-P),
O-9 (F and G worked, all four held blocks surveyed at R1 rock; under `KEY_1` F 271 t in 22 pockets,
G 256/20, J 252/23, K 252/19, pinned by T0), O-10 (the delivery bounded by the mill's 40 t a day,
player ore displacing drone supply) and O-14 (the re-arm rule, §8's note) are the owner's
provisional rulings above. SM.12b's lead defaults, standing until the owner rules: O-11 a receipted
transfer with no haul; O-12 the fee-less century `HouseLicence` (§9.1's note); O-13 the arrival pose
(0, 0), no glyph; O-16 the season measured by T14, never asserted (the drones alone cannot carry a
season); O-17 no staff close or bring-in verb, orders 5a and 5d naming the placement and hand-unload
remedies; O-18 the brief's texts verbatim, `DISPATCHER_NAME` and `HOUSE_LICENCE_NAME` stored on kept
worlds, except order 5d's remedy, which the lead's follow-up `f171b36c` made read `+tv/{n} unload
go` (the bare form is a preview that commits nothing: a remedy that does not work is a defect, not
house voice). **O-15, the drones' soundings:** SM.12b's brief told the owner "about 8, up to 16, 2–3
an IC hour per sounding rig"; that correction was wrong, since `chart::HEARD_EVERY_IC` (3,600 IC s a sounder) was
already at base, and T5 measured 4 heard lines in all and at most one in any IC hour from 16 loud
soundings, so doc 20a §8's original figure (at most one line an IC hour from the house, about 2–4 in
all) stands, the measured figure beside it; the heard rate is measured, never asserted (D29). The
lead's rulings in the plan of record: a stalled rig skipped and named by order 5d (D27); the clerk's
items answering nothing when they cannot progress (D26); the house licence at the org's root scope
(D18, D19); every refused compile backing off one interval (D21); `repossess` stored-only, refusing
a yard hull (D22); the relay seed re-signed at v4 (D24); and R3's wider reach, any parking reason
kept at the bound (§8's note).

**As built (SM.12c-bot):** O-28 default (i), the cohort named in a `ci.yml` comment and run by hand,
is the owner's provisional ruling above, so no machine refreshes the cohort's figures; the canary
runs in every build. O-23, bot funding, kept its default (not among the owner's rulings of
2026-10-24, so the lead's default, standing until the owner rules): no pilot is funded, and at lane
v15's prices each bot paid its licence, lease, deposit, bunkers and stake out of the 25,000 cr
programme stake and netted 735.00cr on 12 t of metal ore, so no staff credit route was used or
built. The lead's rulings the planner took (distinct per-bot configs, the grid read by position, one
family switch while nothing is mined, the window rule, the margins +1 t and +2 crew-days, Ahuna and
Kerwan with Juling the fallback) and the builder's departures (§13's row) stand as the cut note
records them.

**As built (SM.12c-lane):** O-19 (the content bounds raised to 32 and 64, the shipped lane still 8
/ 8), O-25 (the debug pairs: the push-triggered shard holds plain debug ceilings of 500 ms p99 and
2,500 ms max, the tight debug pair asserted only under `TACSIM_ASSERT=1`, release always), O-26
(the Vesta hulls refused at the epoch sky and reported, the loop invariants over the Ceres sites),
O-27 (the month STOPs only at an incomplete mirror or within 3,900 of 4,096; the month's max was
1,117 with (c), against a pre-(c) projection of about 4,060) and O-28 (i) (the month named in a
`ci.yml` comment block and run by hand through `scripts/cost-lane.sh`, no job running it; the
proposed `surface-cost-lane` schedule stanza and the Windows weekly entry exist as text only, and
since the house cron is now daily a weekly run needs a day-of-week guard) are the owner's
provisional rulings above. SM.12c-lane's lead defaults, standing until the owner rules: O-20 the
measured pins over §11's targets, never trimmed (`done.unload` constructed 63 against SM.8's 19
handed, `forget` 5, `claim.stake` 9, `contest` 6, `release` 6, `renew` 5, `unpaid` 6, `lapse` 6,
`seed` 7, `site.open` 5, `site.worked` 6, `licence.buy` 14, `lease.take` 19, `lease.return` 15;
§11's Tick row); O-21 the test-only lever variant, printed against the shipped values, this run's
values in `cost-lane.json`'s `cohort.levers_diff`; O-22 one real hour for the unload's hopper
markers (`UNLOAD_MARKER_KEEP_S`; every other marker keeps the day) and no staff ring-clear verb;
O-24 the drip and the emergency bid reported, not gated, and O-24b none, since `tvpref` writes 1;
O-29 the every-build release gate at ten TVs, the month's p99 by hand only; O-30 six sites, the
lane's. The planner took no lead rulings and named no owner-critical item of its own. **Open for
the owner: O-19c, the economy's per-party live-contract cap at cohort scale.**
`mush_economy::MAX_LIVE_CONTRACTS_PER_PARTY` (32) refuses every Draw and stake once Occator holds 64
leases and the Co-op Trust and the Registry 32 each, so a 64-TV world STOPs at its kit-up; even at
30 TVs, the largest cohort the cap admits, 170 bunker Draws hit the cap transiently, and in play a
third ice sale to Occator settling at once (its 30 rig leases live) is refused "contract exceeds a
cap" and holds its sailing plan until resumed (three in the month after the 2026-10-26 follow-up,
§16). The lead's default, standing until the owner rules: the cohort runs at 30 TVs (`scripts/cost-lane.sh`'s
default) and the cap is not raised by the cost lane; raising it is a mush-economy change, the
owner's to make before any 64-TV month.
**Owner's ruling (2026-10-26):** raised to 100 (`crates/mush-economy/src/contract.rs`), one cap for
players and houses alike, as before. At 64 TVs Occator's 64 leases leave it room for 36 sales and
Draws in flight, and the Co-op Trust and the Registry stand on one contract a pilot each; at 30 TVs
the cap no longer binds in play. Nothing is sized by the cap: each offer counts the parties' live
contracts afresh, and that scan's cost is the store's contracts, not the cap.

- **Q-20-1 What the drill produces.** Default: raw ore goods milled at Ceres (§7.3). Alternative: the
  existing concentrate and water, grade folded into tonnes — simpler, but it makes mine-then-haul the
  dominant play on every charter and bypasses the Exchange's purchase price. This also supplies P5's
  restock half and so touches Q-18-19's "no top-ups", which is why it was the owner's.
  **As built (SM.12a):** the default landed: raw ore is milled at Ceres by the two lane mills (lane
  v14) into the house's asks, in kind (§7.3's note, D-20-10); doc 18's Q-18-19 and D-18-27 carry
  the as-built note.
- **Q-20-2 Tenure.** Default: 640 m blocks with per-block tenure, commons on every body, open ground
  workable until staked, hand-made own-reading evidence to stake, 50 t worked a term to renew, a
  cooling term after a lapse, two live claims a account. Alternatives: whole-site claims (three stakes
  lock Ceres), shared per-pilot permits, a frontier free-for-all with tenure only for permanent rigs.
  **As built (SM.6):** the cooling term is one row per holder on the block's tenure record, at most
  four (§9.2); its "anyone acting for the same principal" half is SM.10's.
  **As built (SM.10):** that half landed with a gateway-account half beside it: the cooling binds
  the holder, anyone who acts for it and every character of the holder's gateway account (OC15, the
  lead's default, standing until the owner rules).
- **Q-20-3 The offline bound.** Default: one landed hold a plan, 288 IC hours, never re-armed, with
  `await tv`. Alternatives: live-only mining; at most eight closed steps and no launch; a plan that may
  cycle to N holds (re-rules Q-18-22 and risks the economy).
  **As built (SM.12b):** the bound holds for every player's plan; the NPC house's plans are
  re-armed under `yard::file_due` (O-14, the owner's provisional ruling of 2026-10-24; §8's note).
- **Q-20-4 Crewed cab or teleoperation.** Default: teleoperated from the settled hull. The crewed cab
  — a body in the cab, exposure, rescue, the Starflight feel — stays open and is a later cut on the
  same TV.
- **Q-20-5 The Co-op's share on mined ore.** Default: a fixed programme reference at half the bid,
  amending D-18-24 for goods no port asks. Alternatives: as built (250 ‰ over the goods table's
  `reference_major`, the logged baseline path; zero only if that is authored at the bid); a flat
  royalty a tonne by a separate contract; exempt.
  **As built (SM.5):** the default landed as the programme's `reference_minor` map (§7.4).
- **Q-20-6 Where the TV comes from, and how many.** Default: Occator leases them (`+tv/lease`, 1,400 a
  term, 5,000 deposit), the Co-op refits the charter onto a Dray-P (`+coop/refit`) with a two-slot bay.
  Alternatives: one slot (a 75 s rhythm and nothing between cycles); the Co-op supplies the TV with the
  refit; a purchase.
- **Q-20-7 Landed life support.** Default: a crew-day a soul an IC day while settled on a site,
  floored at the stores one leg home needs and printed on arrival (a solo week needs a resupply).
  Alternative: none while landed, as today.
- **Q-20-8 The words.** Default `+tv/mine` (alias `drill`), `+tv/scan` (alias `survey`), and `go` as
  the commit word on every `+tv/…` act.
  **As built (SM.7):** `+tv/mine` with the alias `drill`, `go` the commit; `+tv/mine stop` is the
  one form that commits without `go` (R7, an owner-critical default the lead accepted, standing
  until the owner rules).
- **Q-20-9 Soundings observable to rivals.** Default: yes, as a bearing and a range band (one line a
  sounder an IC hour), with a quiet half-radius sounding nobody hears.
  **As built (SM.4a):** the budget is one line an IC hour per (sounder, site), kept across
  `+tv/forget`, and the line reaches only connected listeners — the operators with an open excursion
  on the site and the helm holders of hulls docked there — never queued (SM.4a's D7 and D8, the
  lead's owner-critical defaults until the owner rules); the quiet sounding reads eight cells, as
  proposed.
- **Q-20-10 The second body in phase 1.** Default: a Vesta commons under the Freeport Registry
  (Rheasilvia Commons) with richer metals and the strategic shelf, reached by a seasonal passage, so
  "choose a planet" is a real choice from the first cut. Alternatives: Ceres alone; Phobos first (needs
  a Republic issuer and a Stickney ore bid, phase 2's shape).
  **As built (SM.2):** Rheasilvia is reachable only throttled and in season. A full-tank, empty-hold
  departure plus the leg-home reserve fits the 80 t tank on 0 of 365 days at full throttle and on 191
  at the landing floor, throttle 457 (at the epoch, 64 t out and 37 t held back: 101 t). The lead's
  fix round made the throttle hint reserve-aware, so a pilot at Ceres is told the first day the floor
  fits (123 IC days on, at the epoch); whether the reserve should be cheaper or the site changed is
  the owner's (SM.2's decision 7, the lead's default until then).
  **As built (SM.4b):** nothing new for the question; the Vesta commons (Rheasilvia) passed the
  drive-freeze census like the Ceres sites, 64 of 64.
  **Owner's ruling (2026-10-26):** Rheasilvia being only occasionally reachable is a feature. The
  reserve and the site stay as built (SM.2's decision 7 stands); new, better, faster and bigger
  ships will open it more often when they are introduced.
- **Q-20-11 Prices, the drip and its caps.** Default: `metal_ore` 70, `ice_ore` 35, `carbon_ore` 50,
  `strategic_ore` 600; drips 40 / 40 / 20 / 2 t a good an IC day to standing caps of 240 / 240 / 120 /
  12 t. Lane data.
  **As built (SM.5):** lane v11 at these figures; every drip re-post churns the bid's listing id and
  a lapsed intake returns at one drip (SM.5's decisions 19 and 20, the lead's owner-critical
  defaults until the owner rules).
  **As built (SM.12a):** the mills' rate is the drip's, 40 t of ore an IC day, and the drip and the
  capital, not the mill's resale, size the intakes (§7.3's note; O-1, the owner's provisional
  ruling).
- **Q-20-12 The key.** Default: a fourth durable key family on disk with a fingerprint record and a
  loud disable on mismatch. Alternative: a config seed (silently moves every reserve if it changes).
  **As built (SM.1):** the key is loaded without `?` — a key that will not load is a disabled layer,
  never a refused boot — and only when `[tacsim].enabled`; `[tacsim.geology].key_dir` has no default.
  The shipped template and both dev1 deploy scripts provision `__DATA_DIR__/geology-keys` at mode
  0700, and `docs/ops.md` tells the operator to back it up with the store (the lead's default; whether
  the production box's protected backups include the directory is the owner's operational call).
  **Owner's ruling (2026-10-26):** the geology-key directory is backed up, but no instance takes any
  backups today, the store included. Backup and restore is addressed for the whole instance at once
  as doc 16c's `T-112` (the store, the ledger archive, the quarantine spool and the four key
  families), with the geology key one of its named items.
- **Q-20-13 Cut notes.** Default: `docs/design/20-cut-notes.md`, its own file.

---

## 16. Risks, and what this document refuses

**Risks.** The surface tick shares the engine's 64-actor submit queue with the port tick, and every
surface act now starts there; SM.12's cost lane measures both under load before any site goes live.
The composite LRU can spike CPU when many sites are active; builds stay off-loop and the lane bounds
them. Interpolated TV positions mean a client and the server disagree about an intermediate pose for
up to one fire; harmless while nothing interacts mid-leg, and the promotion phase must derive the
true pose from the leg record. Lanes v9 to v11 (doc 19's A.8 took v8), goods v3, the Dray-P, a new
unit class, a new content kind, two new feeds, a new screen and the grid caps all move pins; each cut
re-blesses, and a kept world needs an operator plan and apply. The geology key is a new operational
secret: it is backed up with its key directory, and a lost key is a disabled layer, not a corrupted
one (as built, SM.1: it is never loaded with `?`, and its directory is provisioned 0700 and backed up
with the store; §15 Q-20-12). Tenure fees are minutes of mining; the worked floor, the cooling term and the finite rich pockets are the conflict,
not the fees. Open ground that is workable without a stake may leave the Registry's fees unpaid by
miners who move on; the stake's value is exclusivity over found ground and the levers are the floor
and the fee. A second TV nearly doubles a pilot's tonnes; the drip's cap, the hold and the lease are
the levers if that proves too strong. Occator's drones and the drip set the ore economy's size; both
are knobs the owner tunes on dev1. The Registry interlock is a lore shortcut that the fight will have
to replace with enforcement play. The Vesta commons is the first site reached by a passage rather
than a hop: its hull is away for days, which the queue's `await tv`, the plan's 288-hour bound and
the licence's anywhere-renewal were sized for, and the cost lane must include it.

**As built (SM.8):** two risks the unload adds. A hopper vault's marker ring holds 128 markers
younger than 86,400 real seconds (about 64 two-ore unloads a real day) before `MarkersFull` refuses
an unload until they age, and the ring is the vault's, so it carries across lessees (SM.11's and
SM.12's rows). A queued unload whose operator lost the helm or the charter is refused at every
attempt — eight tries, one page a boot, then a retry every 60 fires — until any start lands on the
rig; only staff can bring it about, since the repossession and the lease return refuse a loaded
hopper (SM.11's row).

**As built (SM.9):** the clients interpolate (§10), and three residuals the owner should see. The
log pane holds the open excursion only and empties when the rig comes home, since a recover's tell
starts a clean log (D16; the owner may want the last excursion's log kept until the next start, and
SM.11's reconnect summary carries what a sleeping pilot missed either way). The TUI moves a rig only
on its own paints, about every 4 s, so it steps rather than glides; a smoother repaint needs a frame
clock in `server.rs`, the owner's file. The server's gate emitting `rigs[].leg` only for an act on
that site is untested, a rig of the docked hull with an act on another site being unreachable in
the test rig.

**As built (SM.10):** one risk closed, and the residuals handed on. An issuer may amend an Offered
fee back to a draft or re-offer it on other terms; the claim's acts and the clerk read the fee
contract by its whole terms (`fee_terms_hold`), so an altered fee is unpaid and lapses the claim,
never passes for a paid one. The judge's R1 — a landed claim whose fee expired unaccepted standing
free for its term with every reader on `CLAIM_PENDING` — is closed by the lead's fix round (§9.4).
Still open: the heard count's 512-row scan ("at least N", R2); the provenance ring's four entries,
which can evict a challenger's early loud sounding (R3: `heard_before` reads the ring only, while
the holder's discovery survives by `first_local_at`); the paper contract's re-find scanning every
contract in the economy while the claim log lacks its row (R4); a claim order's life,
`CLAIM_ORDER_TTL_IC_SECS`, 3,600 IC seconds or 150 real seconds at rate 24, so an order the world
task does not land in that window is never offered and its fee waits out its hour to refund, and a
`go` after an engine back-pressure drop can escrow a second fee while the first is held (the
licence's behaviour); the claims suite's ~65 s on CI's critical path (SM.12c's row); and a re-stake
after an expired lapse needs a fresh hand-made reading in the new epoch, so `EXPIRED_LAPSED`'s "you
may stake it again" holds only after one. Lane v12 moved the lane's pins.

**As built (SM.11):** the SM.8 note's two risks moved, and the residuals handed on. A queued unload
whose hopper's marker ring is full no longer fails at every try: since the lead's fix round the
surface tick holds its start while the ring has no room (`queued_ring_full`, read through the ring's
own eviction rule, `MarkerRing::admits`), re-reads the ring every 60 fires with no attempt counted
and no page, tells `WHY_MARKERS_QUEUED` once a queue (queued for an offline pilot), and starts the
unload by itself when the ring frees; a queue whose finish was refused `MarkersFull` is re-filed on
the record, and the offer's `unload_markers` refusal is only the backstop for a ring filled under a
submitted start. A plan reads that wait as `Markers`, never a park, and `+tv/plan show` names it;
the build held such a plan until its bound or a clear with nothing re-queuing the unload, and the
round made it retry. A live plan still holds the hand unload (D8). The headroom was measured: the
story's cadence, about 8 markers a landed hold (one every 4.3 IC h), never binds the 128-a-real-day
ring, and about sixteen sold holds in one real day would; the lead's recorded ruling, the ring's
window as the act's retry horizon, stays SM.12c's decision (c). A queued unload whose operator lost
the helm is not dropped under a plan: the plan chain reads it as the plan's own helm wait — told
once, one page a boot under the queue's attempts key — and it clears when a start lands (the pilot
back at the helm, or the next lessee), since no principal can land a drop (the lead's ruling, which
amends SM.8's "drops an orphaned queue with a page" on SM.11's row); a queue orphaned with no plan
stands as the SM.8 note says. Still open: a stranded rig has no staff verb to bring it in, though
`STRANDED` says staff can; an orphaned plan stays on the record, unseen by the next lessee, until
the next filing; a plan's drive runs its A* at the tick and again at the offer (SM.12's row); the
tick's operator-cap census is only as complete as its 1,024-record scan, the prepare's own cap check
refusing the rest; a plan that began parking while its author is ashore waits `Ashore` in the bay
instead of ending, holding the rig `Planning` until the author boards (nominal, since the hull
cannot lift with the pilot ashore), and one whose author is ashore past the deadline ends `Bound`
rather than with its parking reason; and a restart while a ring is still full tells
`WHY_MARKERS_QUEUED` once more. No lane, pack or digest moved.

**Staff recovery follow-up (DF-1):** `@tacsim/tv close|home|unload #<rig>` supplies the
staff remedies missing from the SM.8 and SM.11 notes above. Close cancels work and
releases mining locks in place; home persists a request before returning the rig
to its assigned bay; unload transfers the complete hopper into the docked hull's
hold with the inventory owner's capacity and conservation checks. They can clear
another pilot's plan and an orphaned unload without that pilot's helm or licence.
The authoritative record retains a bounded recovery receipt and its monotonic
ordinal, preventing stale work from completing. Home blocks new work and departure
until its durable finish, including across restart. Leases, deposits and ore are
preserved; marker-ring clearing remains separate. See [ops](../ops.md#staff-rig-recovery).

**As built (SM.12a, SM.12b, SM.12c-bot):** one standing cost, one residual closed and the CI cost.
**The relay seed follows the lane.** Relay R10's `tacsim.relay.luna_ceres` pins the lane at an exact
version (`mush_content::PackRequirement` takes no ranges), so every lane bump must re-sign it at its
next version with its entries unchanged — v3 for lane 14 at SM.12a, v4 for lane 15 at SM.12b — and
the relay code reads any version from 2 as the full star; a kept world's relay latch then stays at
its old version, because staff `@atlas/pack plan` reads "Nothing to do." for an entry-identical
pack, which nothing reads amiss today but which a later "latched requirements consistent" check
would see as skew (doc 15b). **Never-sells, as built:** the dispatcher has no helm,
`ship::no_charter` answers `YARD_STAYS` for every ship verb, the market's `helm_of` gate refuses its
sale (`NAME_SOURCE`) and tacsim's port gate its quote from the host room (`NOT_IN_PORT`,
`port/market.rs`), and `port/yard.rs` names nothing in `mush_market`. The SM.11
note's "one whose author is ashore past the deadline ends `Bound` rather than with its parking
reason" is closed (SM.11's R3): any plan already parking keeps its reason at the bound (§8's note).
The SM.11 residuals SM.12b did not take stay open (the off-helm `mine_stop` widening, the orphaned
plan on a lease's end, the double A*, `hold_figures`' one stowage tag). CI's cost grew: `--test
mill` adds about a minute to the tacsim-integration shard, `--test drones` runs 181–190 s in debug
against its 90 s target, and SM.12c-bot's canary about 9 minutes on the rest shard.

**As built (SM.12c-lane):** the cost lane retired four risks with figures. **The submit queue**: at
most 5 of the 64 actors a fire. **The composite LRU**: C4's eighteen grounds read 54 builds, 38
evictions and 16 entries, and the 30-TV month made 6 builds and 0 evictions; the Draft's "builds
stay off-loop" reads "they stay on the tick, measured" (§3.2). **The Vesta passage**: the hulls
refused at the epoch and reported, never measured in flight (O-26). **The marker ring and the
applied mirror** (O-27): the pre-(c) smoke projected a mirror of about 4,060 against the 3,900
STOP, so decision (c) was built, and with the unload's hopper markers kept one real hour the
month's mirror peaked at 1,117, the rings' least headroom 121, with no `PlanWait::Markers`,
`unload_markers`, `MarkersFull` or back-pressure; the SM.8 note's 128 markers younger than a real
day still holds for every other endpoint. **The month's two findings, investigated (the
follow-up of 2026-10-26).** *The Ceres clerk never starved the mill*: the 177 "idle while a batch
was due" fires were judged after the step that held the port fire, when the step's landings (the
dispatcher's deliveries) and the pilots' sales had already put the ore in the storefront; at the
clerk's own decision no batch was due in any of them, and every one milled at the next port fire
(5 real s). The month now judges a step due as the fire begins, and reads 0. *The load's decay was
the month script, then real depletion*: 33 of the 38 dry loops were metal plans over blocks whose
every fair metal pocket was worked out while 17–33 fair ice pockets a block remained, and the script
switched family only on a refused filing — which a plan over worked-out blocks never was (the new
`NothingLeft` refusal, §8, closes that); it also counted a plan a claim ended as dry. With the
compile refusal, a dry loop marking only its family dry at the site, and only the ground's verdict
(`WorkedOut`) counting, the 30-TV month's loops a pilot rose from 17–40 to 34–54 and three pilots
parked, all after IC day 26, once their loops had found both families worked out at every Ceres
site — at 30 TVs the open ground the plans can file over runs out within the month, metal first
(the month's "sites first dry" figures also carry the hand pilots' own dry calls, so they are no
measure of the ground). *The per-party contract cap binds in play* (O-19c): with 30 rig leases
live, Occator has room for two sale contracts settling at once, and a third ice sale is refused
"contract exceeds a cap: a party is already on the most live contracts allowed" and its sailing plan
holds until resumed (three holds in the month; the script resumes them as a player would). The
month's "stuck" reads progress now (a phase change, or a queued plan's cursor, cycles, trips or
sailing plan moving), not phase age: a plan still mining a day on leaner, contended ground is not
stuck. (The cap was 32 then; the owner raised it to 100 the same day, §15's O-19c, so these holds
are gone at 30 TVs.) A lead item, not fixed here:
`crates/mush-market/src/trade.rs:812` maps any `accept_contract` error, an insufficient balance
included, to "That purchase needs an approval your account rule has not granted yet." (**Fixed by
RK-1:** an escrow the account cannot fund reads `CANNOT_COVER`, any other own-accept error
`BUY_NOT_AGREED` or `SELL_NOT_AGREED`, and only a real approval the approval line.) Still open:
the double A* (measured, not built, §8); the staff
ring-clear verb (O-22); a committed cost-lane baseline and delta step (D-H-8); and CI's cost —
`cost_lane` joined shard 2, its debug wall 105–162 s at default threads and 175 s at two against the
brief's soft 90 s, which only a second binary or shared setup would meet. A build from before
SM.12c-lane cannot decode a hopper ring that carries `keep_s`, so a rollback past the cut meets it;
harmless pre-launch (`docs/ops.md`).

**Refuses.** No RNG anywhere: variety is place, uncertainty is time. No auto-fire and no repeating
loops for a seated pilot. No world-changing bare keys. No arena, window or snapshot on the surface
loop in phase 1. No landing on Mars, Luna or Titan. No grade-priced bids and no graded pools. No
player-owned TV vault. No per-pocket deposit. No cell edits and no depletion features. No new terrain
class and no eleventh commodity family. No surface that reveals truth, the mine preview and the
capacity check included. No submission by the applier or the boot pass. No claim-jumping by drill in
phase 1. No surface combat and no frontier blocks until promotion exists. No mining-only, mind-only or
pilot-only verbs. No deploy without the owner.

---

## 17. The drone phase and the fight (reserved, not built)

**Continuation, 2026-10-28:** [doc 27](27-ic-field-operations-design-and-survey.md) is the
shared DT/H1/H2 design and accepted owner decisions. Units disabled in IC combat retain exact damage and
surviving equipment; a separate physical recovery operation may tow/load them without repair.
The promotion sketch below is a candidate mechanism, not authority to inherit OOC match rules
or make one site's grid/cap universal. An IC engagement intersects the participating units' open
epochs; it is not an OOC match. D-20-7 still describes shipped behavior; accepted Q-H10 authorizes
explicit leave-behind departure in the H1 delivery, without recalling units or closing their epochs.
The global outage freeze/countdown in Q-H11 is a separate world-clock prerequisite; local combat
still cannot hold journeys. H2 towing preserves condition; the explicit post-H2 H3 milestone
combines stripping, scrapping and repair/restoration.

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

**As built (SM.12b):** "repetition arrives only through doc 17" is excepted for the NPC house:
Occator's dispatcher re-files its rigs' fill plans under the re-arm rule (O-14, the owner's
provisional ruling; §8's note), and the seal is untouched.

**The fight.** A frontier block (the Pallas outer claims) where a hostile arrival **promotes the site's
ground into an arena** — the same 256 × 256 grid by digest — behind an IC-crossing ruling that
succeeds D-16-3 and D-UM-50 (what crosses at check-out, what returns at check-in, how a snapshot
rewind cannot double-mint). The TV's class becomes fieldable, `ComponentRole::Tool` and
`DueKind::DrillComplete` mirror the weapon's `ready_at` pattern (`arena/weapons.rs:991-1211`), the
hopper stays world-side under T2, and the excursion `Holds` while the arena holds the TV. Then a TV left
out is a choice, return-on-idle is a survival rule, and claim-jumping is a scene. Nothing in phase 1 is
reworked for it: the site extent, the sortie identity, the tool shape and the knowledge rows are built
to be promoted. Doc 23 §8 keeps this promotion out of its scope and adds only the frame: the match
lies inside the TV's open epoch and cites it (D-UM-46), and D-20-7's departure refusal stands until
this phase (doc 23 D-23-4, Q-23-2).

---

## 18. Amendments owed (made by the cuts that land them)

**Landed.** SM.1 `61324cba` (with its follow-up `6f797974`), SM.2 `bd718fb2`, SM.3a `96cc7303`,
SM.3b `eb99c9be`, SM.5 `64cd1f8e`, SM.4a `354832bf`, SM.6 `70c2cd12`, SM.4b `d367bf3b`, SM.7
`dd48ed45`, AP `74c5801f` (the atlas path search's cost, an engine cut in `mush-atlas` with no §13
row of its own), SM.8 `9922c9c7`, SM.9 `16b0ca4e`, SM.10 `fed09da2`, SM.11 `09854d6a`, SM.11r
`8cc0aa64`, SM.12a `9df93246`, SM.12b `1b5dd371` (with the lead's follow-up `f171b36c`),
SM.12c-bot `d2f4ee7c` (the lead's cohort run on it recorded in `d842fdd0`) and SM.12c-lane
`389383f1` (the cost lane, the train's last cut: the surface-mining train is complete); each cut's
full as-built record is its section of [20-cut-notes.md](20-cut-notes.md), and Drafts 0.5 to 0.14
fold their amendments to this document in as the "As built (SM.*n*)" notes above. None of the
first fourteen edited another design document (they touched `docs/help/ships.md`, `travel.md`, `coop.md`,
`trade.md`, the new `mining.md` — complete since SM.8, its "Still to come" no longer listing the
ground chart since SM.9, its beat 10 built and its staking clauses gone since SM.10, its beat 11
built since SM.11 — `getting-started.md` (SM.8), `orders.md` (SM.11), `docs/ops.md` and, in SM.4b
and SM.9, `tui.md`; AP touched no document), so no item below is closed yet but the
`docs/help/tui.md` item, its "80 columns" half struck below as done by SM.4b and its lettered
screens as done by SM.9, and SM.7, AP, SM.8, SM.10 and SM.11 closed none: an "As built" line under
an item says what the cuts landed in code toward it; the Doc 10b §15.5 item is new (SM.3a), and so
are the Doc 18 item's D-18-27 clause (SM.5) and its planner clause (SM.8). **As built (SM.12a,
SM.12b, SM.12c-bot):** SM.12a touched `docs/help/trade.md` (O-2's sentence), SM.12b no document and
SM.12c-bot `docs/bot.md` (its "Mining pilot"); the Draft 0.13 fold itself folded the SM.12a cut
note's doc 18 items into doc 18 (the P5 clause struck below as done by SM.12a), recorded the relay
seed's coupling in doc 15b, the Mill and Dispatch readiness lines in `docs/ops.md` and an
"Executed" paragraph in doc 20a. **As built (SM.12c-lane):** SM.12c-lane touched no document but
its cut note; the Draft 0.14 fold folded its doc 18 item (the surface unload's hopper markers' hour
under Q-18-74), its `docs/ops.md` item (the cost lane by hand, with the rollback note) and extended
doc 20a's "Executed" paragraph with the train complete. It closed no item below.

- **README**: the doc 20 row, and the doc 19 and 20 rows moved into numeric order after 18a (SM.0);
  rows for `09a/09b/09c`, `11-tacsim-setting`, `16c`, `17e`, `17f` and `18-cut-notes` are missing today
  (SM.0, a drive-by). **As built (SM.0):** the doc 20 row and the numeric order landed; the missing
  rows are still owed.
- **Doc 10 §5** — the loop gains its surface arm: scan → sound → stake → mine → haul → mill.
- **10b §3.3 / §15** — the cut-3 handoffs this document builds (a cadence, a system opener, read
  handles, `ClaimRenew` and the Abandoned take-over, the tenure epoch, `OreDeposit.external`, the
  `ExternalDraws` trait, the contest offer's jurisdiction row, the pocket sidecar) recorded as E7
  residue closed (SM.6).
  **As built (SM.6):** every handoff landed in code — the system opener, the read handles
  (`IndustryHandleCell`), `ClaimRenew` in place, the Abandoned take-over, the tenure epoch,
  `OreDeposit.external`, `ExternalDraws`, the jurisdiction row and the pocket sidecar — as E7 residue
  closed, and the cadence stands as the 5 s port tick; doc 10b's text is still owed.
- **Doc 10b §15.5** — `tacsim.tv.deposit` is the
  first game namespace compiled into mush-economy's frozen `ENGINE_OWNED_RESERVATIONS`, for the
  reasons the `mush-economy/src/api.rs` paragraph gives (SM.3a's D18, the lead's default until the
  owner rules).
- **Doc 11 §4.1, §4.12, §5, §6, §7** — Occator's terrain-vehicle lease beside its drone lease and its
  drone blocks, the Exchange's and Occator's ore intakes, the Freeport Registry as an org with a
  treasury, the Occator claims' margins ruled core until the fight, the Syndicate paper on every outer
  block, the proposed Outer Claims front, Vesta's commons under the Registry (SM.2, SM.5, SM.10).
  **As built:** the Freeport Registry is a Ceres org (50,000 cr, faction freeport) and the issuer of
  all six sites, Rheasilvia included, and Occator holds Occator Rim's inner blocks F, G, J and K
  (SM.2); Occator's terrain-vehicle lease runs eight rigs, `OWP-TV-01..08`, at 1,400 a term and a
  5,000 deposit (SM.3a; doc 11 §7); the Exchange's ore intakes (`metal_ore`, `carbon_ore`,
  `strategic_ore`) and Occator's (`ice_ore`), and Occator's capital raised to 300,000 by an appended
  tranche, landed in lane v11 (SM.5). Doc 11's text is still owed.
  **As built (SM.10):** the Pallas Syndicate is a Ceres lane org (50,000, faction `syndicate`) whose
  paper is seeded on Occator Rim's twelve outer blocks (`occator_outer`, lane v12), and Occator's
  four held blocks are its Uncontested claims under the Registry for a century; the proposed Outer
  Claims front is not built. Doc 11's text is still owed.
- **Doc 17 §12, §22** — the sortie seal at a bay; the `mining` role reserved (phase 3).
- **Doc 18 §4.1 / D-18-24, D-18-27, §4.2, §4.4, §5, §8** — the fixed-reference variant for goods
  no port asks (SM.5); D-18-27's "no top-ups" amended for Occator's appended capital raise (SM.5);
  the Dray-P beside the Dray; `await tv` and `+ship/lift`; ~~P5's supply half lands here (the mill,
  SM.12)~~ (done, SM.12a: landed with lane v14, and doc 18's §5 says so); Q-18-22 extended by
  D-20-11; a site clerk beside the stock clerk on the port tick.
  **As built:** `+ship/lift`, the settled berth texts, `LegPlan.ascent_mps` and
  `PassagePlan.landed_crew_days` (SM.2; §4.4), and the Dray-P beside the Dray, `+coop/refit`,
  `RECALL_BAY` and `BUYOUT_BAY` and every programme hull's hidden bay room (SM.3a; §4.1/§4.2's recall
  and buyout paragraphs) landed in code, and so did `reference_minor` and Occator's appended `raises`
  tranche (SM.5; §7.3, §7.4), and the site clerk on the port tick, on an arm of its own beside the
  stock clerk's (SM.6; §3.4). Doc 18's text is still owed.
  **Owed (SM.8): the planner's cargo mass.** Doc 18's planner prices a passage as laden at
  departure, so the leg home with a mined hold aboard and the pad days fall outside its figures: a
  mining captain who bunkers to the planner's line is a crew-day short for the way home. The SM.8
  story's bunkers carry the gaps measured against the planner's own lines and asserted exactly,
  `STORY_EXTRA_BURN_T` 0 and `STORY_EXTRA_CREW_DAYS` 1 (the laden leg home from Ahuna's pad priced
  at 2 crew-days against the planner's 1; the lead's fix round replaced the build's +2 t and +3
  crew-days). The planner should take an expected cargo mass — an amendment owed to doc 18's
  planner, not to this document.
  **As built (RK-1):** the crew-day was not the ore. A pad stay's charge stops at the leg home's
  crew-days (`landed_charge`, Q-20-7), so the story's measured day was the stay charged against its
  own margin; bunkered to the planner's figures the story needs exactly them, and on Ceres a full
  86 t hold costs Ahuna's leg home nothing more. The propellant is where the cargo mass binds: from
  Rheasilvia a Dray-P's full hold burns 49 t home against the 37 t the filing holds back. `+ship/plan`
  for a site now adds, for a hold that takes ore, the leg home with the hold's unreserved ore room
  aboard (`SITE_HOME_LADEN`, or `SITE_HOME_LADEN_GROUNDED` when no route lifts it). It is advice:
  the filing still holds back the empty-hold reserve (SM.2's decision 7, which Q-20-10's ruling
  keeps). `sm_story` bunkers to it and its `STORY_EXTRA_*` margins are gone; `mush-bot` reads it.
  Doc 18's planner text is still owed.
  **As built (SM.11):** `await tv` landed in code as one closed kind of the captain's queue beside
  its sell, buy, depart and stop (`What::AwaitTv`, wire `await_tv`; `NOT_ORDERABLE` now names it),
  and Q-18-22 is extended as written: a fill plan is one fill of one hold, never re-armed (§8,
  D-20-11). Doc 18's text is still owed.
  **As built (SM.12a):** D-18-26 is amended for milled and opened matter: it enters by `credit`
  under its own receipt with no tranche and no lot lineage (SM.6's opened deposits, SM.12a's mill,
  O-7b, the owner's provisional ruling); and the Exchange's and Occator's asks are restocked only by
  their mills, besides a refund and a fresh listing of the whole pool. Folded into doc 18 by Draft
  0.13: D-18-26's note, the notes under D-18-27 and Q-18-19, and §5's P5 supply half landed.
  **As built (SM.12b):** Q-18-22 holds for players; the NPC house's plan is re-armed and its
  delivery is in kind (§8's and §12.3's notes). Doc 18's text is still owed.
  **As built (SM.12c-lane):** IC14's owner item O7 and E7c2's D1 — the applied marker as the
  replay guard for at least a real day — hold for every marker but the surface unload's hopper
  markers, which keep one real hour (`UNLOAD_MARKER_KEEP_S`, O-22, the lead's default standing
  until the owner rules). Folded into doc 18 by Draft 0.14, as a note under Q-18-74.
- **Doc 19 §3** — `BodyDef.surface_mg`, `radius_km`, `geology`; `LocationDef.lat/lon`; sites as
  chart destinations with their survey words; `+chart/ground` beside `+chart/body` (SM.2, SM.9).
  **As built (SM.2):** `BodyDef.surface_mg`, `radius_km` and an inline `geology` table,
  `LocationDef.lat_mdeg`/`lon_mdeg`, and the sites' survey and tenure lines on `+chart` landed in code;
  doc 19's text is still owed, and `+chart/ground` is SM.9's.
  **As built (SM.9):** `+chart/ground [<body>]` and `+chart/ground map [<body>] [<family>]` landed
  in code beside `+chart/body`, which is untouched, over the `body.ground` feed (§3.1); doc 19's
  text is still owed.
- **Doc 14** — a region placed on its body by `lat/lon` (`mush-atlas/src/apply.rs:427-447` forces
  `parent None` today); the `i16` validator rule and cast; the grid caps (SM.1, SM.4).
  **As built (SM.1):** the `i16` rule landed in `RegionRec::validate` and tightens the boot-load
  contract as well as the apply contract — an out-of-range stored region is refused at load, not
  clamped — which doc 14 should say; the grid caps are SM.4b's. Doc 14's text is still owed.
  **As built (SM.4b):** the grid caps landed in mush-ansi (`GRID_MAX_COLUMNS` 118, `GRID_MAX_ROWS`
  32); doc 14's text is still owed.
  **As built (SM.12c-bot):** doc 14 §1's P10 ("Text is sufficient") is met by the four-bot SSH
  cohort in line mode (§13's SM.12c-bot row); doc 14's text is still owed.
- **Doc 16** — nothing changes; §12 gains a note that D-20-1 keeps the arena to fights and that the
  promotion phase will bring the IC-crossing ruling.
- **`docs/help/tui.md`** — ~~the lettered screens `c` and `q`~~ (done, SM.9: the intro and the
  `c chart` and `q quarry` entries); ~~"drawn for 80 columns" corrected~~ (done, SM.4b: line mode
  reads "drawn for 120 columns", a map at most 118 columns by 32 rows).

---

## 19. The owner's review (2026-10-19), and what it changed

Eight findings, all confirmed against the tree and the text, folded in Draft 0.4:

1. **A full hold stranded the departure.** Seven hoppers left 2 t of room; the eighth left 10 t in
   the TV and `HOPPER_LOADED` refused the lift. Now no cycle starts without room for its yield in the
   hold's **unreserved share** — the hold's room less every hopper aboard and every outstanding cycle
   of every rig — so a recover always unloads everything and the plan parks when no cycle fits (§6,
   §7.1, §8, D-20-9). As built (SM.4b): the unreserved room is the `site.map` frame's `hold.room_t`,
   the hoppers' loads counted and the outstanding cycles SM.7's (§10). As built (SM.7):
   `hold_unreserved` counts every outstanding cycle of every rig of the hull, in every measure (§6).
   As built (SM.8): a recover does unload everything after an honest fill; a hold staff have shrunk
   unloads what fits and says what stays, and the departure names the unload (§6, §4.3).
2. **A pre-worked commons failed the invariant at once.** The opener credits exactly the initial
   reserve, so a `worked_permille` recorded as drawn quarantined the block on the first sweep. Now the
   reserve is opened gross and the worked share is a receipted transfer into the site's spoil with its
   cursor rows, so `initial − pool == Σ drawn` holds from the start and the matter is accounted for
   (§3.4). The same finding exposed that the applier may not submit at all (doc 18's refusal), so the
   opener is a **site clerk** on the port tick in the stock clerk's shape (§3.4, D-20-5, SM.6).
3. **Expired rights blocked recovery.** Re-checking licence and tenure at every act could strand a
   loaded rig and contradicted "knowledge is free". Rights are now **per act**: licence and tenure
   gate mining, staking and renewal only; driving, scanning, sounding, recovering, unloading and
   dumping need the control lease and the settled hull; a plan whose rights lapse recovers, unloads
   and parks with its reason (§8, D-20-11). As built (SM.3b): the control lease is read as the live
   lessee aboard the settled hull (§4.1; the lead's default, standing until the owner rules). As
   built (SM.11): a plan re-reads the rights at every step's start, a sounding's and a drive's too,
   and a lapse parks it with its reason: it recovers, SM.8's queued unload empties the hopper and
   the plan ends, told once as it parks and once as it ends (§8).
4. **Charge was committed apart from the work.** The mine start reserved its lock and cycle
   authoritatively but the charge was a later tick write, so a scan could spend the same margin. Now
   **every act starts as an authoritative submission** that reserves its charge on the excursion's
   state family, and the normal-lane document is a projection (§4.1, §8, D-20-6, D-20-11, SM.3).
5. **The walkthrough mined open ground the gate forbade.** Open ground is now **workable under a
   licence until someone stakes it**; a stake buys exclusivity over found ground, which is the
   decision (§1, §6, §9.2, D-20-12; the one default moved after acceptance, §15).
6. **No rig or family selection.** A leading slot number names the rig on every verb; `+tv/mine
   [family]` and `+tv/plan … family=` choose and freeze the family; `await tv [<slot>|all]` tells
   waiting for one rig from waiting for every rig, and `depart` refuses while any is out (§1, §6, §8,
   §10, D-20-13).
7. **Blind mining leaked richness through the capacity check.** A blind cycle now needs 3 t of room
   in the hopper and the hold, the most any class yields, so fitting or refusing never tells the class
   (§6, D-20-9).
8. **Lapsed claims had two ways round the worked floor.** A lapsed holder may not re-stake the block
   for one full term (the cooling term), and contest evidence must be a heard sounding stamped inside
   the block's **current tenure epoch**, which every lapse and take-over advances (§9.2–9.4, D-20-12,
   SM.10). As built (SM.6): every lapse, take-over and transferring seizure advances it, once per
   block event (§9.2). As built (SM.10): the contest window runs from the block's last opening
   (`opened_ic`), and a take-over's or a transfer's own epoch advance never narrows it (D18, §9.4).
