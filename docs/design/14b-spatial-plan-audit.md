# 14b — What the spatial plan said, and what was built

An audit of `14-spatial-implementation-plan.md` against the tree at the S9 landing
(2026-09-10, `origin/main` after the S9 commit), cut by cut: what each cut's scope and
tests paragraph promised, what its ledger and the code say landed, and the gap — kept,
changed with a reason, dropped, or still open. The plan's ledgers are the primary record;
this document is the cross-cut view the ledgers do not give, and the list of what is still
owed. Doc 14 (Draft 0.1) is the design the plan implements; where the plan itself diverged
from doc 14 the divergence is in the plan's §6 decisions and is not re-audited here.

Reading the columns: **said** is the plan's text before the cut; **built** is what the
ledger records and the code shows; **gap** names each difference and whether it is *by
reason* (a deviation the ledger argued, kept as the better design), *withdrawn* (the plan
was wrong to promise it), or *open* (still owed, with the owner's call if one is needed).

## 1. The one-line scorecard

| Cut | Said (§1 scope line) | Built | Gap |
|---|---|---|---|
| S1 | `mush-content` (packs, digest, latch with update/retire) and `mush-atlas` (records, rasterizer, text views, `@atlas/pack`); starter packs at first boot | All of it | The tacsim loader was not moved (a new owner-independent loader instead; the fold came at S7); atlas records ride the compatibility lane, not a state family — *by reason*. Interface abilities left for S6 — *kept as planned* |
| S2 | One travel system replacing `route.rs` and `journey.rs`; `+travel`, `+stakeout`, holds, timed portals, the tour bus | The travel system, `route.rs` deleted, `+travel`/`+stakeout`/`+intercept`, timed portals, the bus | `journey.rs` layered, not replaced (D-P-4, the owner's amendment); 22 tests, not ~46 — *by reason*. The fold promised for S7 was withdrawn at S9 (D-14-35) — *withdrawn* |
| S2b | Theme, weather, control regions and property on the atlas | All of it | `Region.atlas` a string id, `Property.site` a `RecordKey`, weather keyed on pack ids; 14 tests, not ~20 — *by reason* |
| S3 | The building agent and hydration: tool loop, validator, proposals, staff and player paths, caps and strikes, reconceptualize | All of it | Reservations are the request records, not a `ReservationBook`; re-entry through the existing `ModuleAsync` seam; `file` not an in-game source; 28 tests, not ~52 — *by reason* |
| S4 | Windows, the street driver, rendering: tiles, features, floors, composite, pose, resolver, write-back, `+map`, the panel in text and on the web | All of it | Composite re-entry through the atlas's own completion, not a core arm; no separate 200 cm outdoor tiles; the TUI's map is the unit screen's pane; elevation digits and room/neighborhood-scale feed views not built — *by reason* / *open* |
| S5 | Engine physics: the locomotion table, sight, fire and beam traces, hearing; propagation on the metric | All of it | Cells of ruleset classes not built (still open after S9); traces single-surface (fixed at S9.4 for doorways); `path_max_cells` read by no command — *open* |
| S6 | Interface NPCs: the provider seam, asker-scoped authority, the starter cast's abilities | All of it | The trait in `mush-core`, not `mush-npc`; the answer spoken, not fed back; the audit ring the NPC module's — *by reason*. Replies broadcast to bystanders — a defect the review found, fixed at S8 |
| S7 | The TacSim ground driver, heightfield import, the grid vocabulary removed (D-14-4), the campaign ephemeris; the corridor's records and latch onto the atlas | The sky, the driver, the import, the grid words gone; the loader folded | The latch and the corridor's records deferred to S9 (D-14-33); the fire gate deferred to S9 — *by reason, then closed at S9* |
| S8 | The review's five, with the review's tests passing unchanged | All five, the five tests unchanged, twelve beside | None. The bot crate exempted from the panic limits at the owner's word — outside the cut |
| S9 | The seed latch and the corridor's records onto the atlas; the carrier journey fold; the fire gate; the two S7 leftovers | The corridor as sites and connections through the content latch; the fire gate; the line through a doorway; the carrier journeys read by the atlas | **The carrier fold withdrawn** (D-14-35, the owner may overrule); **cells of ruleset classes still open**; the legacy seed path kept for content-less compositions — *withdrawn* / *open* / *by reason* |

## 2. Cut by cut

### S1 — `mush-content`, `mush-atlas`, the starter packs

**Said.** Move the tacsim loader unchanged in bytes with an entry codec registry; the latch
as a generalized `SeedApplication`; `mush-atlas` records verbatim from doc 14 §2 in a typed
state family; the rasterizer; `+atlas` and `@atlas/pack`; boot applies the starter packs
under `[game].starter_area`; ~44 tests; lenses P/I/G/B.

**Built.** A *new* pack layer digesting raw canonical JSON (owner-independent identity), an
`EntryApplier` per kind registered by the atlas and the NPC host; a per-entry latch that is
honest about a partial apply; objects and the latch in one boot batch; atlas records as
bounded documents on the compatibility lane; `ContentModule` composed **last**; the boot
packs gated by config, not a testkit flag; tiles by a `finish` pass with measured encodings
(256×256 stands); the two neighborhood packs ordered by `requires`.

**Gap.** The "three tacsim digests unchanged through `mush-content`" test was not written —
the loader was not moved, so there was nothing to hold unchanged; the fold and the re-pin
came at S7 (*by reason*). The state family was not used (*by reason*: tiles take part in no
authority; the allowlist names the family they would move to). `TestWorld::with_starter_area`
does not exist (*by reason*: a testkit→atlas edge would cycle). Everything else landed as
said, with the verify fold's four findings fixed before landing.

### S2 — one travel system

**Said.** `JourneyRec` *replacing* `CarrierJourney`; `route.rs` and `journey.rs` deleted;
`@route` and `@journey` removed; timed portals through a `MoveAttempt` hook; stake-outs,
intercepts, clocks; ~46 tests; lenses N/C/S/K.

**Built.** `JourneyRec` as the atlas's own record (characters, objects, room-model vehicles);
`route.rs`'s runtime deleted; timed portals queued by the hook and departed by the tick;
the transit room and the patch as runtime rooms; the tour bus as a room that travels;
`+intercept/release` and `@atlas/journey send` beyond the list; knowledge by visiting; 22
tests covering every named bullet; six verify findings fixed.

**Gap.** **`journey.rs` and `@journey` were not deleted** — the owner amended D-P-4 at S2
("layered, not replaced, until S7's fold"), and the fold was withdrawn at S9 (below). The
intercept and the door-hold are RAM (*by reason*, noted). Test count lower (*by reason*: one
engine, two fixtures).

### S2b — theme, weather, control regions, property

**Said.** Theme on the records with `resolve_theme` ending at `#0`; weather cells keyed by
atlas id; `Region.atlas: Option<CanonicalId>`; `Property.site: Option<CanonicalId>`; ~20 tests.

**Built.** As said, with a world-theme document beyond `#0`, `PlaceRef` as the shared lookup,
`Region.atlas` a string (localities too), `Property.site` a `RecordKey`, the legacy zone cell
dropped loudly; 14 tests.

**Gap.** None owed. Control drawn on the map (deferred to S4's projection) was not drawn at
S4 either — *open, minor*: `+territory` prints the extent.

### S3 — the building agent and hydration

**Said.** `mush_llm::agent::run` with `RunBudget`; the validator as a library; `ProposalRec`
in a typed family with `WorldCommand::ProposalUpdated`; a `ReservationBook`; the `BUILD`
queue; caps and strikes; `@atlas/propose` from prose, file or `--from`; ~52 tests.

**Built.** The loop over a `ToolHost` (the NPC turn rides it); the validator over a captured
context; requests as the reservation; the existing `ModuleAsync` re-entry; `AtlasHooks` for
the hand-off; the jobs module's status-observer seam; packs from the shelf, not files; 28
tests, every named scenario covered.

**Gap.** The proposal family is not a K state family (*by reason*: the atlas takes no
authoritative action yet; the first K-owned hold brings the family). `file` as a propose
source dropped (*by reason*: packs arrive on the shelf). Nothing open.

### S4 — windows, the street driver, rendering

**Said.** `WindowRec`, `Pose`, the composite at `mod:tactics:map`, write-back at tick
boundaries, 100 cm indoor and 200 cm outdoor tiles, `WorldCommand::WindowComposited`,
`+map`/`+move`, the panel feed with a text renderer registrar, the TUI map screen, the web
canvas; ~54 tests.

**Built.** All of it with: the composite through the atlas's own async completion; the
window document under the atlas's namespace, immutable per revision; the resolver as a slot
the atlas fills; windows sampling S1's tiles (no 200 cm tiles); the text fallback shape-driven
in `mush-ansi`; the TUI map as the unit screen's grid pane; immediate write-back for street
windows; rasterizer folds for doors and floors.

**Gap.** **Elevation digits on request** and **room-scale and neighborhood-scale views on the
feed** not built — *open, minor* (`+atlas` keeps its own text map). A room's access stood in
for the exit's lock on a walk "until S5's table" — S5 priced the walk but did not gate it; the
review caught the bypass and S8 gated it.

### S5 — engine physics

**Said.** The vocabularies, the table as data, `move_cost`, `path`, the four traces, hearing,
`resolve_room`; propagation on the metric; ~40 tests; lenses D/O/X.

**Built.** All of it: costs in hundredths with a climb rule, tags amending rows, the geometry
slot in `mush-sim`, `@atlas/los`, twenty tests.

**Gap.** **Cells of ruleset classes** — a cell is one byte of a closed enum; the table takes a
ruleset's classes by name — deferred to S7, then S9, **still open**. **Traces single-surface**
— fixed at S9.4 for a doorway (same-height junction); a line down a *stair* (a portal with
`dz`) is still not traced — *open, minor*. `[atlas].path_max_cells` read by no command — *open,
minor*. Elevation digits — as S4.

### S6 — interface NPCs

**Said.** The provider trait in `mush-npc`, `AbilityContext { npc, asker, principal, powers }`,
eight providers, the cast's `interfaces`, `@agent/set`, ~36 tests; lenses A/M/$.

**Built.** The trait in `mush-core` with `AbilityCall { npc, asker }` and the context
re-actored; the answer spoken; the audit ring in the NPC module; the rules interface as a
small module; `who.nearby` as `look`'s answer; `weather.forecast` from the transition table;
`market.quote` writing the asker's own quote; the packs at version 3; eighteen tests.

**Gap.** "Advertised" presence for `who.nearby` awaits 14c (*open, by design*). **Replies
were broadcast to bystanders** — the review's P1 #2, fixed at S8 with a per-reply audience.
The Gretchen/control-region authority test became the Susie/Production-Office test (*by
reason*).

### S7 — the ground driver, imports, the sky, the grid words gone

**Said.** 1000 cm windows; the ruleset's locomotions from the table; indirect fire wired to
the pulse driver's intents; heightfield import; the corridor's records as atlas sites and
connections and the seed latch onto the content latch; the grid vocabulary removed; the
campaign ephemeris; ~26 tests; lenses D/P/H/E.

**Built.** The sky (circular, authored, pinned at load, the drive's distance law), the loader
fold with re-pinned fixtures, heightfields inline, the ground driver as `DomainDriver` with the
domain as its window, the arc answered, the grid vocabulary gone under a contract test;
twenty-eight tests.

**Gap.** The **latch and the corridor's records** deferred (D-14-33) — *closed at S9.1*. The
**fire gate** answered but not wired — *closed at S9.3*. The **multi-storey line** and
**ruleset-class cells** — S9.4 (one closed, one open). The multi-storey line through a
*stair* remains open (S5's gap). `nominal_drive` picks the law by class name until cut 11's
propulsion table — *open, TacSim's*.

### S8 — the review's five

**Said.** Five fixes at the seams the review named, the review's tests unchanged, one or two
cases each; lenses V/A/L.

**Built.** As said: `authorize_traversal`, `ReplyAudience` (D-14-34), `+map` from the frame,
staff authority at apply, NPCs on the ground window; the five reproductions unchanged, twelve
cases beside; the lens census in the ledger.

**Gap.** None owed. Beside the cut, the bot laboratory was measured then exempted from the
panic limits at the owner's word (out of the engine); the review reproduction's state-slot
write baselined.

### S9 — the deferred four

**Said.** S9.1 the content latch as the latch with the corridor's records as atlas sites and
connections in an orbital/system region (D-14-33 defaults); S9.2 the carrier journey absorbed
by `JourneyRec { traveler: Carrier }` through a `CarrierMotion` slot, `journey.rs` and
`@journey` retired, the carrier suite migrated; S9.3 the fire gate once the subject exists;
S9.4 the doorway line and ruleset-class cells; lenses P/E/S/T/O/D.

**Built.** S9.1 as recommended (three appliers, the region entry, positions from the
ephemeris, the corridor's length the pin, `<pack>@<version>` seeding, the legacy path kept for
content-less compositions); S9.3 in the tick beside the contact guard's witness; S9.4's
doorway line; for S9.2 the seam — one route source, the atlas listing and answering for
carrier journeys — with the record and the verb left where they are.

**Gap.** **S9.2 withdrawn** (D-14-35): the census showed a 1,786-line placement-graph state
machine with 142 references that the atlas's room-model journey shares no semantics with;
folding it is a slot of half a dozen callbacks and a hundred rewritten tests for record unity
alone. The owner may still call for it; the ledger records the cost. **Ruleset-class cells
still open.** The **legacy seed path** stays (*by reason*: E7's suites compose the ruleset
alone; a legacy-seeded world refuses a re-seed through the content latch rather than making
its places twice — the migration is by hand).

## 3. What is still owed, in one place

| Item | Owed since | Owner's call? | Size |
|---|---|---|---|
| Cells of ruleset classes (an open terrain enum through the rasterizer, compositor, frame, TUI and web legends) | S5 → S7 → S9 | No — but no driver needs it yet | Medium |
| The carrier journey fold onto `JourneyRec` (D-P-4's promise) | S2 → S7 → S9 | **Yes** (D-14-35: the plan recommends leaving two records) | Large |
| A line down a stair (a portal with `dz`) between floors | S5 | No | Small |
| Elevation digits on the map; room- and neighborhood-scale views on the feed | S4 | No | Small |
| `[atlas].path_max_cells` read by a command (NPC walks) | S5 | No | Small |
| Advertised presence for `who.nearby` (14c) | S6 | Out of this plan's scope | — |
| Control drawn on the map | S2b → S4 | No | Small |
| Migration of a legacy-seeded world onto the content latch | S9 | No — by hand when one exists | Small |
| `nominal_drive` by class name (cut 11's propulsion table) | S7 | TacSim's | — |
| The hired car (doc 07 §9) on S2's journeys and S6's abilities; 14c presence; 14d faction places | §9 of the plan | Out of this plan's scope | — |

## 4. What the plan got wrong about itself

Three things, worth saying plainly because the ledgers say them only cut by cut:

1. **Test counts were estimates, and the estimates ran high.** The plan promised ~44, ~46,
   ~20, ~52, ~54, ~40, ~36 and ~26 tests for S1–S7 and landed 40-odd, 22, 14, 28, (S4's count
   is in its ledger), 20, 18 and 28. Every *named* scenario was covered; the counts fell
   because one fixture exercises one engine, and because several named bullets share a test.
   The plan's numbers should have been scenario lists, which they became from S8 on.
2. **The plan promised folds it had not costed.** The tacsim loader "moved unchanged in bytes"
   (S1) became a new layer with the fold at S7; the carrier journey "replaced" (S2) became
   layered, then deferred, then withdrawn. Both were the right outcomes; both should have been
   a census first and a promise second. S9's ledger is the first place the census preceded the
   cut.
3. **The plan deferred to dependencies that were already there.** S7 sent the fire gate to
   "E7k's unit family or the K6 contact guard, whichever lands first"; the K6 guard had
   landed, and its witness was in the tick's hand at one line. A read of the tick before the
   deferral would have landed the gate at S7.

And one thing it got right that is worth keeping: **the review's tests as the cut's tests,
unchanged** (S8) — a contract the plan should apply to every future review.
