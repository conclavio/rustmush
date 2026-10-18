# rustmush design documents

The first five documents were produced by parallel design passes (2026-08-27) and reconciled by a
critique pass. They are the *reference* design; where they conflict with each other, the rulings in
[`../PLAN.md`](../PLAN.md) ("Treaty rulings") are binding.

| Doc | Area |
|---|---|
| [01-server-core.md](01-server-core.md) | Object model, world task, command dispatch, module API, scheduler |
| [02-connectivity.md](02-connectivity.md) | SSH (russh), session layer, web server, WS protocol, frontend |
| [03-llm-integration.md](03-llm-integration.md) | Provider adapters, budgets/audit, worldgen pipeline, NPC agents |
| [04-game-systems.md](04-game-systems.md) | Ruleset API, dice, WoD, TacSim seams, comsys, mail, weather, jobs |
| [05-critique.md](05-critique.md) | Cross-doc contradictions, gaps, risks, milestone plan |
| [06-object-behaviors-spatial-domains.md](06-object-behaviors-spatial-domains.md) | First-class object behavior, nested spatial relationships, simulation domains, and the TacSim implementation path |
| [07-npc-control-and-tactical-ai.md](07-npc-control-and-tactical-ai.md) | Link-loss automation, allegiance, typed orders, hybrid tactical AI, belief state, and NPC mobility |
| [08-economy-organizations-ownership.md](08-economy-organizations-ownership.md) | Cross-ruleset economy, first-class organizations, scoped authorization, asset ownership/custody, contracts, housing, and production |
| [08a-economy-build-status.md](08a-economy-build-status.md) | **Build ledger for doc 08**: what of E0–E8 is shipped (commits), the decisions made while building, the live seams, and the E3 pickup point — read first when resuming economy work |
| [proposals/e7-production-slice.md](proposals/e7-production-slice.md) | **Proposal (input to 10b, not authoritative):** the K side's E7 requisition/production vertical slice, its pulled-forward K4a/K5a dependencies, migration gates, models, milestones |
| [09-rules-engine-contracts.md](09-rules-engine-contracts.md) | Twelve shared mechanical contracts, authoritative action transactions, and five minimum cross-ruleset proving gates |
| [08b-e3-implementation-plan.md](08b-e3-implementation-plan.md) | E3 implementation plan (Draft 0.3, as built + verify outcome): holds-free custody, approvals, existence hiding, faction linkage |
| [08c-e4-implementation-plan.md](08c-e4-implementation-plan.md) | E4 implementation plan (Draft 0.5, as built through E4c + verify folds; §12 doc 09 conformance and the K3 review) |
| [08d-e5-implementation-plan.md](08d-e5-implementation-plan.md) | E5 implementation plan (Draft 0.2, as built through E5b): WoD housing, careers, stances, prompts; owner answers binding |
| [08e-e6-owner-questions.md](08e-e6-owner-questions.md) | E6 owner survey (answered): the Shadowrun run-and-pay economy, 24 binding decisions |
| [08f-e6-implementation-plan.md](08f-e6-implementation-plan.md) | E6 implementation plan (Draft 0.3, re-grounded at ce43714 after the look-back: consent by authorship, issuer = payer, attestation-only E6a, rolling approval window, D-E6-19..23): run and pay, outcomes/fences/custody, the neutral market crate |
| [08g-e1-e5-look-back.md](08g-e1-e5-look-back.md) | E1–E5 look-back (Draft 0.1): 2,127 claims audited, 32 confirmed gaps ranked, the handoff chain, standing debt, doc 08 vs shipped, K integration, the single fold list (§11), owner decisions (§12) |
| [08h-e5-interim-review-response.md](08h-e5-interim-review-response.md) | E-side response to the K side interim E5 review (Draft 0.2, refinements folded): five findings adjudicated, six questions answered, decisions D-LBK-1..7, the fold table for the E5/K6 gate |
| [08i-e-k-dependencies.md](08i-e-k-dependencies.md) | E ↔ K dependencies and overlap (Draft 0.1, on the merged main): no K blockers for E6/E7 cuts 1–11, 29 needs by cut, 18 overlap surfaces with mitigations, the anti-debt rules read against E's build, sequencing with three sync points, ownership lines under the owner's rule, 16 questions for the K side |
| [08j-convergence-review.md](08j-convergence-review.md) | E ↔ K convergence review (Draft 0.1, on the merged main 4c80014): 503 claims from both sides verified against the merged tree, 17 surviving gaps ranked, the gate ledger, K8.4/K8.5/K8.6 compatibility, cross-tree seams, the fix-before-E7 list, doc amendments owed |
| [10-tacsim-foundations.md](10-tacsim-foundations.md) | TacSim foundations (Draft 0.5): scope, decisions 1A–7A and C1A–C30A, addenda T1–T17 (T17: the frozen sky — distances derived from real geometry at one epoch and pinned), sharpenings R1–R7, the E7 backlog |
| [10a-tacsim-e7-owner-questions.md](10a-tacsim-e7-owner-questions.md) | E7 owner survey (Draft 0.2, answered): 22 decisions, the eleven-cut structure, handbacks to E5/E6 |
| [10b-e7-implementation-plan.md](10b-e7-implementation-plan.md) | E7 implementation plan (Draft 0.2, defaults accepted): the authoritative eleven-cut plan over the landed K4a/K5a primitives |
| [11-neighborhood-geography-and-authoring.md](11-neighborhood-geography-and-authoring.md) | Neighborhood-first geography, zoom vocabulary, local/regional travel, tactical alignment, play discovery, and versioned worldbuilding briefs ([template](neighborhoods/TEMPLATE.md), [urban example](neighborhoods/examples/lantern-ward.md), [rural example](neighborhoods/examples/mill-creek.md)) |
| [11a-neighborhood-geography-review.md](11a-neighborhood-geography-review.md) | **Review of doc 11** (2026-09-09, on `0bc5e92`): every reuse claim read against the tree, the un-cited seams (`CanonicalId`, the E7 pack + seed latch, `RouteDef`/`CarrierJourney`, C8 reservations), the terrain layer that does not exist, findings H1–H6/M1–M11/L1–L7, the one-substrate proposal (rooms, terrain, elevation, floors, connections, ships across scales), the hydration lifecycle tightened, a revised build order, decisions D-11a-1..13, and the amendments owed to doc 11, the template and the examples |
| [11-h-implementation-plan.md](11-h-implementation-plan.md) | H implementation plan (Draft 0.2, reviewed): four cuts of low-level work beside E7 and I — the module store-read seam (H1), the K7 production NPC cutover (H2a–c), authoritative-lane instrumentation and the scale harness (H3), the kernel-edge debt sweep (H4) |
| [12-economy-checkpoint-review.md](12-economy-checkpoint-review.md) | The economy checkpoint review (C17): where a cold replay's time goes, the checkpoint designed, the owner's decisions (D-12-1..6), and the cuts as built — J1 parse once, J2 the bounded RAM tail, J5 the ledger archive tier, J3 the checkpoint, J4 the narrow fetch |
| [13-idempotency-window.md](13-idempotency-window.md) | The idempotency window (D-12-6): every key on disk forever, a bounded window in RAM, a scalable Bloom filter gating the off-loop lookup with a transient refusal on the rare positive; cuts W1 (mechanism) and W2 (window) |
| [14-spatial-room-system.md](14-spatial-room-system.md) | **The integrated spatial/room system (Draft 0.1, 2026-09-09)**, written from first principles off 14a's answers: the model and its invariants (rooms on one metric substrate, layered 2.5D elevation, tiles and features, portals, region-owned connections, one journey record, windows over the map, the resolver), the records and the JSON pack schema with the starter area as content, boot and apply, travel and stake-outs, windows and write-back, the building agent's contract and validator, hydration with caps and strikes, engine-owned physics versus ruleset definitions, SSH-first map rendering, the starter lot's content (Susie, Phil, Gretchen, Karen Smith, Regina, Cady Heron, Janis Ian, the tour bus), interface NPCs (§4.6, decided), six build cuts, supersessions, decisions D-14-1..32 |
| [14-spatial-implementation-plan.md](14-spatial-implementation-plan.md) | **Implementation plan for doc 14 (Draft 0.12, 2026-09-10; S1 through S9 landed, their ledgers under §3.1–§3.10; the plan-against-build audit is 14b)**: seven cuts — S1 `mush-content` and `mush-atlas` with the starter packs at first boot, S2 one travel system, S2b the atlas adopts theme/weather/control/property, S3 the building agent and hydration, S4 windows and the street driver with text-first rendering, S5 engine physics, S6 interface NPCs, S7 the TacSim ground driver and the last hexes — each with records, verbs and their allowlist lanes, wiring, config, tests and verify lenses; build discipline, constants, decisions D-P-1..10 |
| [14a-spatial-owner-questions.md](14a-spatial-owner-questions.md) | **Owner survey (answered 2026-09-09, all recommendations plus seven sharpenings)** for doc 14, the integrated spatial/room system proposal to be written from first principles: the givens (premises 1–6, rulings R1–R4, the no-spend boot rule), what the tree has, 29 questions with recommended options across scope, what space is, elevation and terrain, movement, domains, authoring and the building agent, the starter area, and what moves onto the atlas; the facts needed (I1–I9); the defaults taken unless vetoed; the outline of doc 14 |
| [14a-spatial-s1-s7-code-review.md](14a-spatial-s1-s7-code-review.md) | **Code review of the S1–S7 implementation (2026-09-10, at `d713d17`; changes requested)**: five defects reproduced as failing `review_*` integration tests (a tactical step bypassing exit locks, interface replies broadcast to bystanders, `+map`'s text naming hidden participants, a demoted author applying a staff proposal, NPCs left out of ground engagements) and the four documented deferrals of the S7 ledger; the plan's S8 fixed the five (landed 2026-09-10, the review's tests passing unchanged) and S9 folds the four |
| [14b-spatial-plan-audit.md](14b-spatial-plan-audit.md) | **Audit of the spatial plan against the build, S1 through S9 (2026-09-10)**: cut by cut, what the plan said, what landed, and the gap — kept, changed by reason, withdrawn or still open; the list of what is still owed (ruleset-class cells, the carrier journey fold under D-14-35, a line down a stair, the feed's coarser views); and what the plan got wrong about itself |
| [15-comms-relay-backbone.md](15-comms-relay-backbone.md) | **Communications (Draft 0.1, 2026-09-12; owner decisions D-15-1..11 binding, engine backlog R1–R10 not started)**: the quantum relay backbone — paired links, light-speed last mile, the lag table, the Luna star at world start, the Malapert Exchange (service classes, the congestion board, Served/Throttled/Suspended), the six blackouts and the nuclear option, private pairs and the first half afloat as end-game, the doc 10 §3 amendment, seven open questions |
| [16a-combat-owner-questions.md](16a-combat-owner-questions.md) | **Owner survey (Draft 0.1, 2026-09-16, answered and closed)** preceding doc 16: the owner's single-game direction and Option C, the standing rulings kept and the three amended, what the tree has (no integrated motion, no weapon model, three-cell detection, empty catalog), the two acceptances, 40 questions with the owner's answers inline (arena task per map with its cost sheet, adaptive 500/1,000/5,000 ms tempo, RAM-only OOC matches, integer `Pose`, no-dice resolution, rulesets deleted first, 2.56 km arena), eight facts needed, and sixteen ratified world facts |
| [16-combat-arena.md](16-combat-arena.md) | **Combat requirements (Draft 0.2, 2026-09-16, revised after review; decisions D-16-1..20 binding)**: the two acceptances (the two-pilot story test on the production composition with the observer rule asserted surface by surface, the 300-unit harness), the sixteen world facts as engine requirements, the arena as an authority with an eight-message conceptual seam (custody in, intents, control, stop, ledger, frames, custody out, deliberation signal), adaptive tempo made rate-independent by one-cell substeps and timestamped events, integer space and one Euclidean range, persistent per-pod M32 tanks with a defined wreck, scheduled impacts and per-mille resolution from dispersion against silhouette with labelled draws, the damage chain, the shadow-contact perception model and the world-side exclusion of a deployed arena, pods, stations on the tank, `BOARD` and bare cockpit verbs, the lifecycle with its named triggers and boot reconciliation, streamed ledger and records, AI reflexes and the deliberation hook, the `tacsim` game crate, tests, the six extension acceptances, supersessions including the retirement of the `+fire` lane |
| [16b-combat-implementation-plan.md](16b-combat-implementation-plan.md) | **Implementation plan for doc 16 (Draft 0.2, 2026-09-16, reviewed)**: build discipline, thirteen cuts (R-a hoist and composition move, R-b ruleset deletion and gate re-bless, G configuration plumbing, P integer Pose, H harness per D-H-8, A arena core with the cross-tempo test, T arena task, seam, custody and the old lane's retirement, C content with the OOC pack at version 4, K cockpit and lobby, S perception, W weapons and the story test, N reflex AI and the cost run, X extensions) with steps, verify lenses and sizes, the constants table, the cut ledger, the three-lens review with all 35 findings dispositioned, risks |
| [proposals/tacsim-history-retreat-and-concordat.md](proposals/tacsim-history-retreat-and-concordat.md) | **Accepted proposal (2026-09-12, twelve rulings, folded into doc 10 §3, doc 11 §1–§3, doc 15):** Earth 2040–2270 — the Retreat, the Triage, the Interval and its pockets (Luna as the space-technology pocket), the Convergence, the Concordat of 2268 and its Charter (the Warrant, the Compact, the outbound mandate), the Outbound; why humans are still in the loop; the defensibility table |
| [17-unit-minds.md](17-unit-minds.md) | Unit minds specification: observation parity, rules, cores, seals, authoring, proving and the Roll; M1–M5 and L landed |
| [17b-unit-minds-implementation-plan.md](17b-unit-minds-implementation-plan.md) | Unit minds build ledger and baselines; current continuation and performance direction point to 17d |
| [17c-m5-design-note.md](17c-m5-design-note.md) | M5 design and acceptance record, including the two explicit post-M5 follow-ons |
| [17d-unit-minds-through-m6-plan.md](17d-unit-minds-through-m6-plan.md) | Implementation plan through M6 (Draft 0.1, 2026-10-17): rule language, M5r, authoring, proving and bench tools; M7 waits for an economy-stability review |
| [18-trade-run-vertical-slice.md](18-trade-run-vertical-slice.md) | **The trade run, the first IC vertical slice (Draft 0.4, 2026-10-17, as built through IC12; D-18-1..51, every Draft 0.2 default binding under the owner’s rulings R1–R9, Q-18-21 reversed; Q-18-32..41 still open and Q-18-42..65 raised by the cuts, each with the default the tree took)**: a pilot walks from Limbo into Ceres Freeport, signs the Belt Carriers’ Cooperative’s First Captain programme (a chartered Co-op Dray under `Encumbrance::Lease`, a 25000.00cr stake, a quarter of the trading margin to the Co-op inside every sale, a 90000.00cr buyout), loads a hold that binds on 100 t **and** 120 m³ from finite receipted stock, queues the captain’s orders and logs off; the ship burns, flies the Ceres–Kirkwood–Stickney lane in about twenty hours at rate 24 under the slice’s constant sky, docks itself, and the queue sells to Kasei, buys crated yard fittings and alloy stock and departs again at the pilot’s hour; the verified map A–R, the design, the Dray definition, fourteen cuts — **IC0–IC12 landed** (IC11 at 26e3b1fa; IC12, the story test, help, readiness and this docs pass, with Draft 0.4), **IC13 (the buyout) next**, IC14 the residuals — and the two-day story test, built in IC12 on the production root (`mush_server::boot::build_engine`, every leg through the action coordinator, three restarts); each section carries “As built (ICn):” notes citing the tree; the live smoke against Redis is still owed |
| [18a-trade-run-stations-and-ship.md](18a-trade-run-stations-and-ship.md) | **The trade run’s places (content brief, Draft 0.1 revised with doc 18 Draft 0.4, 2026-10-17, as built through IC12)**: Ceres Freeport (13 rooms, three decks; shipped at `neighborhood.ceres_freeport` v2) and Stickney Yards (12 rooms, two decks; v3) room by room with themes, details, exits, items, the NPC cast with voices and fallbacks, lore pools and ambience; the Dray’s six-room interior and its stowage in the same voice (`tacsim.pack.dray` v2: figure-free prose, every figure a gauge line), its hold read in tonnes and cubic metres; the Yard’s crated fittings and the Co-op’s bid for them; how the brief becomes atlas packs; the names accepted under Q-18-12; “As built (ICn):” notes where the shipped packs differ (seven detail keys respelled, verb meta-text struck, the status line’s five forms) |

`06-object-behaviors-spatial-domains.md` and its later companion proposals are
implementation-informed drafts produced by dedicated design conversations deferred in `PLAN.md`.
For their subject areas, prefer them over the earlier sketches while they proceed through review.

## Superseded decisions

Nine choices in these docs were overridden after review (owner's call) — read the docs with these
substitutions in mind:

1. **Persistence is Redis 8+, not SQLite.** 01-server-core §5 (rusqlite/write-behind/snapshots) and
   every other doc's SQLite reference are superseded: Redis is the authoritative persistent store
   (RedisJSON object docs, Query Engine indexes, streams, TTL keys) with **write-through**
   per-command MULTI/EXEC pipelines via the `fred` client. The in-process `World` remains a full
   read cache; crash recovery = reload from Redis. See PLAN.md "Persistence" and "Redis data layout".
2. **Scripting is Rhai, not mlua/Luau.** 01-server-core §8's mlua recommendation is superseded;
   the sandbox/budget/hook design carries over onto Rhai's built-in limits
   (`set_max_operations`, call-depth and size caps). Script API namespace is the `mush` Rhai module.
3. **Worldgen digs.** 03 §10 item 5 ("worldgen digs rooms/exits as well as describing them" —
   rejected, "revisit as a `@gen/plan` proposal-only feature") and the "never invents topology" rule
   of 03 §5.2–5.3 are superseded by the owner's ruling of 2026-09-09
   ([11 §11 R1](11-neighborhood-geography-and-authoring.md#11-owner-rulings-2026-09-09-binding)):
   the building agent proposes topology, objects, NPCs and hooks; a deterministic validator checks;
   humans review after activation where policy says so. The standing principle is
   **trust-but-verify** — a restriction whose only purpose is to keep an LLM from doing what a trusted
   builder may do is not load-bearing, and is struck when it blocks the model. `help worldgen`'s
   "describes; never digs" stays accurate for the shipped `@gen` until the builder lands.
4. **No hex maps; square grids everywhere.** 04's "hex maps with terrain types and elevation per
   hex", 06 §3.3's "hex-map terrain and coordinate data", §6.1's `tactics.hex`, §10's "N-hex radius"
   and "hex-by-hex", and the tactics seam stub's hex vocabulary are superseded (owner, 2026-09-09,
   [14 D-14-4](14-spatial-room-system.md)): a hex cannot be shown honestly in a text terminal, and
   the terminal is the floor for every map.
5. **The engine defines the world.** 06 §3.3's "terrain-cost and kinematics maths to the ruleset"
   is superseded (owner, 2026-09-09, [14 §5](14-spatial-room-system.md)): terrain classes,
   locomotion classes and their movement effects, line of sight, projectile and energy propagation
   are engine concepts; rulesets register definitions — units, weapons, damage — that consume them
   and may extend the vocabularies.
6. **One game, no loadable rulesets.** 04 §3's ruleset plugin API and every "cross-ruleset" framing
   are superseded (owner, 2026-09-16, [16a §1.1](16a-combat-owner-questions.md)): the codebase is
   the 2350-era TacSim game; the platform provides concepts and closed behaviour vocabularies, the
   game's config provides the specifics (the ten commodity families, units, weapons, sensors) as data;
   `mush-ruleset` is the platform's sheet library with one implementation; `ruleset-wod` and
   `ruleset-shadowrun` are deleted (16b cut R). Where "the ruleset registers" appears in docs 09 and
   14, read "the game config registers".
7. **One writer per authority.** 06 §6.3's "no task or channel per unit or domain" is amended (owner,
   2026-09-16, [16 §3.1](16-combat-arena.md)): a running combat arena is its own authority with its
   own task and an adaptive pulse; the world task and an arena share nothing but the eight conceptual seam
   messages of 16 §3.1 (custody in, intents, control, stop, ledger, frames, custody out, deliberation signal).
8. **The sky will move, after the first IC slice.** Doc 10 §22 T17's frozen sky ("no body moves",
   `rate = 0` the ruling) and 1A's rejection of simulated ephemerides are superseded (owner, 2026-10-15,
   [18 R9](18-trade-run-vertical-slice.md)). After the trade-run slice an astronomy engine (doc 18 cut
   P-A) moves real three-dimensional bodies on the IC calendar, so route distances, durations, fuel and
   prices depend on when and where a ship sails. Until then the slice flies a **constant sky** through
   the same seams the engine will use — one sky query, one passage planner whose output every departure
   freezes (T1, load-bearing again) — and no slice figure is an invariant. 1A's travel scales and
   compression and "transfer conditions modify routes, never close destinations" stand.
9. **Two travel records, one hard boundary.** D-14-35 is decided (owner, 2026-10-15, doc 18 Q-18-6):
   `mush_sim::CarrierJourney` carries hulls across the corridor and the atlas's `JourneyRec` carries
   characters within a region; the route source is the tacsim lane pack, and the drawn `spacecraft`
   connection is a projection of the ephemeris at the epoch that no verb traverses and no code reads as
   a travel path (`spacecraft` leaves the atlas self-modes in doc 18 cut IC1).
