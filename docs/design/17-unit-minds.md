# 17 — Unit minds: Rhai on the hull, the shot decision, the core, the lease and the Roll

**DECIDED, 2026-10-02.** Written from the proposal
[`proposals/unit-minds.md`](proposals/unit-minds.md) at `1125c834` and the owner's rulings of
2026-10-02 on its open questions (R-A to R-D below; every other recommendation of the proposal's §21
accepted as written). The argument — the three angle designs (engine, player, economy), the
feasibility pass over the code and the three judges' resolutions — stays in the proposal; this
document is the living spec. Its decisions, D-UM-1 to D-UM-55 (§20), are binding; §21 records the
owner's answers and the few questions the rulings raise (Q-UM-12 to Q-UM-16, each with a default);
§24 collects as-built amendments the way doc 16 §11 does. Nothing here changes shipped behaviour
until §22's cuts land.

The owner's seven pins of 2026-10-02 stand: P1 the shot decision and no auto-fire; P2 minds are Rhai,
sliders first; P3 one code path for AI fill, performance permitting; P4 sealed before the unit
leaves (refined by R-B: sealed per sortie, from the startup in a hangar or base to the power-down in
one); P5 a lease with a hardware requirement; P6 minds are tradeable; P7 the REPL. So do the lead's
architecture pins, restated for any unit: the observation is exactly the unit's station view (the
cockpit's for a combat hull); the outputs are the unit's closed station verbs through the same
resolvers; think at N and land at N+1; an op budget, never wall time; randomness from the sim's
seeded draw; source hash, engine version and parameter block in the sortie's seal record (carried
by the `MatchRecord` in the arena); tiers are lore facts that bound engine cost.

Parents: doc 16 D-16-2 (rate-independent tempo, due events), D-16-3 as amended at 16b cut AF (the
durability rule: OOC match state in RAM, the match ledger streamed through the world task, the arena
snapshotted to the store every `snapshot_ms` and resumed from it) together with doc 16's framing of
the arena as an OOC construct, which D-UM-49 and D-UM-50's own no-IC-crossing rule extend (D-16-3
itself says nothing about IC posting; its row gains that clause when doc 16 is next amended, §19),
D-16-4 (integer state,
keyed draws, a hash per substep), D-16-6 (M32 is the unit truth), D-16-8 (per-mille resolution, the
percentage is shown), D-16-9 (closed consequence vocabulary), D-16-10 (contacts are an in-RAM
shadow with their own id space), D-16-16 (the reflex controller); W5, W7, W8, W12, W16; doc 11 §2
(the Warrant and the Compact; habitat discipline); the Retreat proposal §8.4 and §8.5; the F3
control-capability, channel and lease bundle in `crates/mush-sim` (stations grant channels,
commands require them, a per-subject control-epoch lease); 16c T-54 (per-channel leases unbuilt).

**The owner's rulings (2026-10-02).**

- **R-A, Q-UM-4 reversed to neither A nor B.** The OOC sim is wholly OOC and crosses no IC boundary,
  IC money included. In the OOC sim a character has exactly the minds they hold IC — their licensed
  lines, their own minds and charters, the Registry's Doctrine — at no cost, and none they do not
  hold, so that no IC-OOC knowledge leaks either way. No licence is ever bought for the sim; the
  author market is IC only. An IC **Ceres Simulation Center**, a mixed-use training and recreation
  simulator where IC payouts may ride on simulated fights that consume no real IC units, is named as
  the future IC venue running the same machinery (not designed here). D-16-3 stands, and doc 16's
  OOC framing is strengthened by D-UM-50's own no-IC-crossing rule. → D-UM-49, D-UM-50, D-UM-51.
- **R-B, the seal generalised to the sortie.** A match is one reason to board a unit. A lance may go
  out to mine (one armed escort and AI miners), or nobody may be armed (hauling cargo across the
  map); the end state is large IC region maps where units go out, mine, haul, fight and build, return
  to base, empty their cargo, sell or refine. The unit is sealed at its startup before it leaves the
  hangar or base and stays sealed until it returns to a hangar or base and powers down. Nothing in
  this document may assume a unit is armed, in a match, or on the arena map. → D-UM-45 to D-UM-48, and
  rewrites of §1, §3, §4, §5, §9, §10, §11, §12, §13, §16, §20 and §22.
- **R-C, Q-UM-5 A with a differentiation.** A mind may never order, but NPC lance-leaders are people
  — AI-played characters — and give orders. Only a character principal, human- or AI-played, issues
  `ORDER`; a unit mind is never a principal. W7's deliberation layer is reframed as an NPC
  lance-leader ordering under their own warrant through the existing `ORDER` path. → D-UM-2 and
  D-UM-11 (amended), D-UM-52, D-UM-53.
- **R-D, Q-UM-8 A with a condition and a hook.** Player minds may fly NPC organisations' hulls when
  the player has released the mind through a listing (sealed or source-provided). And a player may
  discover an organisation running their mind unlicensed by recognising a quirk of its behaviour in
  the field: an earned-evidence channel for the contraband loop. → D-UM-54, D-UM-55.

Reading order for someone new: §1 (the shape and the sortie), §4 (the station verbs), §5 (the fire
law, which is where P1 lives), §6 (scheduling, which is where determinism lives), §9 (the tiers and
the pool), §20 (the decisions).

A word on words. *Pilot* below means the human at a unit's stations, whatever the unit does; *hull*
means any unit, armed or not; *sim task* means the deterministic task that runs a map — today only
the arena, later one per IC region (the combat-scale shape of a sim task per region). Where a
paragraph speaks of the arena, a match or the cockpit's combat verbs, it names the special case.

---

## 1. The shape: three goods, one seal, one ladder

A unit mind is a sealed Rhai program that runs on a unit's compute core, reads exactly what that
unit's stations show a human at them, and acts only through those stations' own verbs. The unit may
be a tank in a match, an escort on a mining run, an AI miner working a claim, or a hauler dragging
cargo across a region map with nobody aboard armed; the mind is the same machinery in each. Around
the program sit three goods and one record.

The **core** is hardware: a component on the hull with a Compact class. Every hull has a core slot,
whatever its role. It sets what any mind on that hull can do — how often it thinks, how many ops a
think may spend, how many contact rows and (on a hull with weapons) odds it is shown, how many
condition slots and memory keys it holds (P5, §9).

The **licence** is law: a term lease on one filed mind, naming a holder, a seat count (concurrent
sealed sorties) and a liable warrant principal. A governed core refuses at the seal any mind the
licence does not cover (P5, P6, §10).

The **charter** is authorship: a titled object on the Registry's Roll that earns the licence revenue
and can itself be sold (P6, §16).

The **seal** binds hull, core, licence, revision and the parameter block into one immutable record
for a **sortie** (D-UM-45): taken at the principal's dispatch that sends the unit out of a hangar or
base, ahead of its first startup, held until the unit powers down in a hangar or base (the same one
or another). In the arena the pod is the hangar: `BOARD` then `STARTUP` is the hangar exit, the match
lies inside that one sortie, the match's end (custody returning to the pod) is the hangar return that
closes it, and the `MatchRecord` carries the sortie's seal. On an IC region map the sortie is the
trip out and back — mine, haul, fight, build, return, unload — and the unit's black box opens it
with the seal (P4, §13).

The owner and the players will say *mind*. The help says once what the fiction says: a unit mind is
not one of the Compact's frontier-class minds (the Retreat proposal §8.5: few, licensed like
reactors, located, audited; in play only as advice to human offices). It is habitat discipline's
automation (§8.4): it does, it does not decide; it is legible, overridable and maintainable; it runs
under a named principal's warrant; it is never itself a principal and never orders (D-UM-52). That
is why sliders come first *in law* and not only in the interface (§11): a filed mind must expose its
discretion as a declared parameter block, and the principal warrants the mind by setting those terms
and, on an armed hull, arming its condition slots. The Rhai body is the engineering; the card is the
warrant's terms.

Everything a player meets is one **ladder** with five rungs, and every surface — the card editor,
the station view's `MIND` block, the proving ground, the bench — serves every rung. Rung 1 picks a
labelled preset for the unit's role. Rung 2 moves sliders on that preset's card. Rung 3 writes
condition rules (on a combat hull, `FIRE cannon WHEN odds >= 60 AND aspect = side,rear`). Rung 4
writes a Rhai `think` that calls the role's stock mind (`stock::think` for the combat role) and
overrides one part. Rung 5 writes a mind from scratch with its own card schema. A new pilot never
needs a rung above the one they are on; an author never leaves the ladder to reach the top. The
ladder, the cards, the presets and the profiles are per role (D-UM-48): Standard Doctrine and its
six presets are the combat role's, the first that exists; a mining or a hauling role brings its own
stock mind and presets with its stations.

The engine spine: a mind is a pure function
`mind(view, memory, card, keyed draws) -> (at most 16 intents, memory')`, run by a `MindRunner`
trait the sim task calls, on a sandboxed op-only Rhai engine that lives outside
`crates/tacsim/src/arena`. Its intents are the unit's station verbs under the F3 channel model
(D-UM-47): helm, sensors and fire today, mining, cargo and building later; the cockpit's combat verbs
are what the helm, sensors and fire stations of a combat hull offer. The player angle supplies the
surfaces (the ladder, the rule language, attribution, the proving ground, the bench). The economy
angle supplies the law (the warrant, the governed core, the Roll, charters, the evidence-only crime
loop). Where the three contradicted each other the judges' resolutions are taken; the argument is in
the proposal.

---

## 2. What already stands, and what the designs got wrong about it

The feasibility pass read the engine. These facts bound the design, and each angle design asserted
at least one of them falsely; the corrections are folded in below and named here so a later writer
does not re-import them. Every row is a fact about the arena, the only sim task that exists; a region
map's sim task inherits each of them or names its divergence in §24.

| Standing item | Where | What it gives this document |
|---|---|---|
| The reflex is a staggered due event, `DueKind::ReflexTick`, every `reflex.decide_ms` (1,000) of arena time, resolved at its own millisecond; a per-pulse controller would break D-16-2 | `arena/reflex.rs` head; `step.rs:1508` | The scheduling pattern a mind must copy (§6) |
| `Arena::order` turns a plan into intents stamped with the current epochs, through `apply_intent` with `IntentSource::Machine`, resolved at `t_ms` by `resolve_startup`, `resolve_lock`, `resolve_fire`; its Emcon arm is a no-op | `reflex.rs:4567-4640`; `intents.rs:603` | The door minds land through, factored out (§4) |
| The arena's `Channel` is a closed, hashed enum of three members, `Helm` (heading, speed, startup), `Sensors` (contacts and locks), `Fire` (weapons); `Channel::ALL` is "the set a cockpit station grants"; the arena's `EpochTable` is keyed `(unit, Channel)`. F3 in `mush-sim` is a separate vocabulary: stations grant open `ChannelId` names by set membership (`StationSpec.channels`, "names are ruleset doctrine"; `MOVEMENT_CHANNEL = "movement"`), commands require them, and a lease maps `(unit, ChannelId)` to the principal holding it | `state.rs:2962-2980`; `mush-sim data.rs:866-872`, `seat.rs:73`, `commands.rs:2652` | The output set is the station verbs, a channel each; a mind holds no lease (§4, D-UM-47) |
| An `Order` carries `issuer: Option<ObjId>`, **the unit whose scope a target hint is written in**: the ordering pilot's own tank, or `None` for the squad's commander, which is the lowest unit id in the squad and "not a role" (`ordered_target` resolves the hint in that unit's contact store); the production `None` sites are the deliberation seam's `order_of` and the bridge's `OrderWatch` | `deliberation/order.rs:292-300`; `deliberation/request.rs:506`; `arena_bridge.rs:3904`; `deliberation/squad.rs` head; `reflex.rs:3118` | `issuer` stays the hint scope; the character-principal check needs a field of its own, `by` (§4, D-UM-52, D-UM-53) |
| Every random value is `draw(seed, &DrawLabel)`; there is no stream and no generator state; `LABEL_KINDS` is pinned at five by the contract test | `rng.rs:1-22, :186`; `arena_library.rs:92` | Mind randomness is a sixth label, not a stream (§7) |
| The arena library gate bans eight substrings (`mush_store`, `tokio`, `HashMap`, …) in `arena/`; `rhai` is not among them but the interpreter still must not enter the library | `arena_library.rs:41-80` | The `MindRunner` seam (§7) |
| Substeps are cut from each pulse's own start; a late production wake ends a pulse off the lattice; pilot notices and held-loss settles are staged after the substep hash; `pending_notices` is cleared every pulse and must be empty in a snapshot | `clock.rs:575`; `step.rs:1586-1660, :1898, :3099`; `snapshot.rs:374` | Why "collect at substep end before the hash" is wrong and why minds carry their own notice buffer (§6) |
| Epochs are keyed `(unit, Channel)`; only `apply_control` raises one, then calls `set_controller`; `set_controller(Pilot)` calls `reflex.release`, which deletes the row; Linkdead hands nothing to the machine, only `Released` does | `state.rs:3042`; `intents.rs:470-555`; `reflex.rs:845, :1464` | Possess, release and the dormant mind (§12) |
| A three-channel cockpit seat holds no K6 lease; per-channel leases are unbuilt (T-54); the seat rate is one counter per unit, for `Seat` only | 16c T-54; `intents.rs:713-727` | Pilot-plus-mind channel splits are deferred (§12) |
| `Consequence` is closed: `DestroyUnit`, `RemoveWeapons{section}` (weapons only), `Slow`, `NoTurnToward`, `DisableMotion`; only `wreck_ops` revokes capabilities, and only power, mobility and weapons; `UnitMutationOp` has no install op | `definitions/unit.rs:275-335`; `damage.rs:940-970`; `mush-tactics unit.rs:1169-1240` | Core damage needs two new consequences; fitting needs an op (§9) |
| `Frame` carries contacts, lock, weapons, sections, heat, receiver, ECM, notices and catch-up; it has no standing-order field and no arena-time field; `FrameWeapon.p_permille` is against the lock only; `resolve_seen` reads the target's silhouette off the live unit | `step.rs:85-196`; `weapons.rs:550`; `resolution.rs:512` | The keyframe parity rule and the classification gate on odds (§3) |
| The reflex reads past the observer rule in four places: the `ObjId` tie-break in `best_hostile_where`, `reflex_has_line` tracing to the true pose, `ordered_target`'s Support branch reading the charge's own row or lock, and `contact_reads_dead` reading `target.fatal_sections` | `reflex.rs:3118-3290`; `damage.rs:493`; `perception.rs:1806` | The named divergences of the port (§17) |
| A unit that cannot move and carries no weapon is inert to the machine; the harness's non-fighting units and the range target are hulls without a fire station in practice | doc 16 §6 (16b cut O.3) | Standard Doctrine must run on an unarmed hull (§17, D-UM-48) |
| `mush-script` builds `Engine::new()` (the full standard package: `timestamp()`, float math, print to stdout) and its `on_progress` samples `Instant::now()` every 1,024 ops; `strikes_to_halt` is 3 | `mush-script/src/runtime.rs:567-610, :584-597` | Not reusable for minds; the three-strike rule is kept (§7, §8) |
| rhai is `{ version = "1", features = ["sync","serde"] }` at the workspace, resolved 1.26.0; every knob named in §7 exists in 1.26.0; `rhai::Map` is a `BTreeMap`; float `+ - * /` are built-ins with no package | `Cargo.toml:81`; rhai 1.26.0 `api/limits.rs`, `api/options.rs`, `func/builtin.rs:155` | The sandbox (§7) |
| The harness checks out **all 300 units** as `Controller::Reflex`; the 24 "pilots" are frame sinks; the lens is 12 ms per wake on both cohorts and 12 (CI) or 20 (authored) ms per 500 ms window; the baseline reads wake p99 6.55 ms, window p99 10.56 ms, one substep per wake, four wakes per arena second | `harness.rs:1897`; `harness_baseline/arena.windows.json` | The budget arithmetic is for 300 machines, not 276 (§9, §18) |
| A micro-benchmark on the blessing host (rhai 1.26.0, release): 19.5–24.1 ops/µs on mind-shaped code; 15.05 µs per think with custom types against 24.69 µs with maps at 12 contacts; 3.67–3.93 µs flat from 4 to 48 contacts when scoring is a native kernel; compile plus seal 270 µs; a zero-contact think 0.99–1.34 µs | the proposal's bench output (`scratchpad/minds/rhaibench/bench-output.txt`) | Every op number in §9 (§18) |
| `Representation::Entitlement` ("licences, programs"); `Legality` is recorded, rendered and filtered on, never enforced; `CatalogSource`, `AvailabilityProvider`, `DeliveryProvider` are wiring-frozen; `InstrumentKind::Entitlement` is *documented* non-transferable and unenforced; `issue_title`, `PayoutShare`, `DefinitionDigest::for_canonical_value`, `extraction_authorized`, `Lot.lineage`, `Delegation.expires_at` exist | `mush-market record.rs:125-215, module.rs`; `mush-economy record.rs:124, api.rs:2498, contract.rs:126`; `mush-core mechanics.rs:257`; `mush-industry licence.rs:156`; `mush-inventory record.rs:227`; `mush-org record.rs:212` | The market and the law reuse shipped seams (§10, §16) |

---

## 3. The observation: `MindView`, a projection of the station view

**The parity rule (D-UM-4).** A mind reads exactly what the unit's stations show a human seated at
them, and nothing else. For a combat hull in the arena that is the `arena.cockpit` keyframe; for a
hull whose stations are helm and sensors only, it is the same keyframe without the weapons block;
for a future mining, cargo or building station it is that station's view, which enters the keyframe
in the cut that adds the station (D-UM-47). A tier may truncate the view; nothing may extend it.
Every field a mind reads is first a field of `step::Frame` and of the keyframe, with a
`FRAME_VERSION` bump, so the observer rule holds by construction: the view is built only from the
per-platform builders a pilot's frame is rendered from — `frame_contacts` (`step.rs:2883`),
`frame_scope` (`:2909`), `frame_sections` (`:2977`), `Arena::frame_weapons` (`weapons.rs:936`) and
`lock_aim` (`resolution.rs:814`). A new `Arena::mind_view(&mut self, unit) -> Option<MindView>` (the signature as built by 17b cut
M1a.5, §24) calls those builders through `frame_for`, factored out of `frames` by M1a.3. It cannot reuse `Arena::frames` (`step.rs:1821`), which is `&mut self`,
pilot-only, once per pulse, and drains `pending_notices`.

**One formatter.** `frame_contacts_held`, `contact_band`, `ContactRow` and `own_scope`
(`cockpit.rs:824, :1041, :1062, :963`) are pure functions of a `Frame` plus a range memory and move
into `arena/view.rs`, together with `fired_by_default`, `class_word`, `relative_mdeg` and
`held_range_m`, which they call. `CONTACT_RANGE_STEP_M` and `lock_range_step_m` already live in
`arena/perception.rs:2127` and stay there. `cockpit.rs` re-exports, so `CONTACTS`, the panel rows
and the mind read one function. The per-pilot range hysteresis lives on the world mirror
(`CockpitFrame.ranges`, `arena_bridge.rs:576`) and settles once per frame sent; a mind's twin,
`MindState.ranges`, settles once per think and is folded into the hash. The rounding is the same
function; the cadence of its memory differs by design, and the twin fixture does not compare them.

**Three fields join the frame and the keyframe (D-UM-5)** — for pilots and minds alike, since this is
what "the engine supports the shot decision" means in P1. Two of them exist only on a hull with a
weapons station; an unarmed hull's view simply lacks them and loses nothing by it:

- *aspect*: the word `facet_band_seen` (`resolution.rs:306`) gives off the contact's own `SeenPose`
  (front, left, right, rear), advice from the plot as the reflex's exposure term already is. Aspect
  is shown on every hull, armed or not: a hauler's driver reads which way a contact faces too;
- *odds* (weapons station only): one per-mille per weapon against any **classified** contact
  of another side (as built, 17b cut M1b.3, Q2: the hull's lock first, then nearest-first; a friendly row and a
  friendly lock print the dash; §24), computed by `Arena::resolve_seen` (`resolution.rs:512`) against a new `Arena::aim_at(platform,
  contact) -> Option<LockAim>`, which is `lock_aim` generalised to any held contact over
  `fire_cover_at` (`damage.rs:520`, a trace to the plot cell). Never for an unclassified contact:
  `resolve_seen` reads the silhouette off the live unit (`at.profile.band`), so an unclassified
  contact's odds would reveal its class. Only for the first `odds_rows` contacts the core's fire
  control resolves (§9); `CONTACTS` shows a dash where the core shows nothing;
- *reads_dead* (weapons station only, since it rests on a kill or a fire-control reading): **not**
  the reflex's `contact_reads_dead` (`perception.rs:1806`), which reads `target.fatal_sections`, a
  fact about the machine. The view derives it from what the platform earned: its own gunner's kill
  notice, or a held `FrameCondition` reading that shows a fatal section destroyed. That is 16b cut
  E.2's rule applied honestly, and it is a named divergence of the port (§17).

On the 120-column `CONTACTS` row of a combat hull the two new columns read
`BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon 62%`; an unarmed hull's row ends
at the aspect. The bot's `Picture` parser and the mush-ansi goldens change in the same cut.

**Two more fields join the keyframe** because minds need them and pilots may as well have them:
`order` (the standing order from the unit's character principal or lance-leader, with any target hint
already re-keyed to the unit's own `ContactId`, §4) and `t_ms` (sim milliseconds, the only clock a
mind has). The player angle's "course-and-speed estimate" is **not** added: it is a derived input no
pilot sees, so a tier would be extending the view, which D-UM-4 forbids. A mind that wants a track
keeps its own `last_plot` in memory, as the stock mind does.

**Notices.** `pending_notices` is cleared every pulse and must be empty between pulses, so a think
that spans pulses cannot read it. `MindState.notices` is a bounded buffer (16 entries) filled by the
same post-hash journal scan that stages pilot notices (`stage_mind_notice` beside
`stage_pilot_notice`, `step.rs:1586`; as built by 17b cut M1a.5 the one `stage_notice` pushes to the
mind's buffer at staging time, §24), hashed and carried by the snapshot, drained at each think.
Refusals of the mind's own landed intents are appended to it, so a refusal is never silent (D-UM-8).

**Contact identity (D-UM-6).** The API names contacts by the opaque `ContactId` the sim task holds
(D-16-10's own id space), with the designator and alias carried as fields. An integer id costs no
string ops, survives a callsign rename or alias, and cannot be forged. The trace and the debugger
print designators.

**The Rhai types** are registered custom types with getters, never Rhai maps: the bench read
15.05 µs against 24.69 µs at 12 contacts (D-UM-7). `MindView` holds the unit's own facts (callsign,
team, cell, pose, speed, power, held, wreck, immobile, sight, ECM, receiver, heat, sections with
facets; on a hull with weapons, the weapons with slot index, ready, `ready_in_ms`, rounds, bands,
heat per shot, range band and `p_permille` against the lock; the stations the seal grants), the lock
and, armed, its odds, the standing order, `t_ms`, the drained notices, and `contacts` truncated to the
core's row count, nearest-first by held range with `ContactId` as the tie-break. Each contact carries
id, designator, class word, friendly, bearing and relative bearing, range and band as `ContactRow`
rounds them, age, cell, the earned condition bands with the frame's `scan_tables` (the section words `condition.table` indexes, a per-view index never stable across thinks; added 2026-10-04 after review, finding 4), aspect, and on a hull with weapons odds per
weapon (within `odds_rows`) and `reads_dead`. A future station's fields (a cargo hold's fill, a
drill's yield) join the view with the station, under the same parity rule.

**The map is public.** Map knowledge reaches a mind only through native kernels over the map's
public definition — in the arena `ArenaDefinition` terrain, zones, waypoints and key spots, which a
pilot already receives whole on the `arena.map` feed (`cockpit.rs map_frame`); on a region map
whatever that map publishes to a driver, and no more. No kernel takes or returns an `ObjId`.

---

## 4. The outputs: the station verbs, the motor, and a verb for pilots too

A think emits intents by calling registered functions that append to a native per-think buffer of
at most 16 entries; a 17th is a fault. The buffer is the think's return. Nothing touches the sim
until landing, and a faulted think lands nothing, so a think is atomic (D-UM-9).

**The output set is the unit's station verbs (D-UM-47).** Under the F3 model in `mush-sim` a unit has
stations, each station grants channels by set membership (`StationSpec.channels`, open `ChannelId`
names that are ruleset doctrine), every command requires its channel, and a lease maps `(unit,
ChannelId)` to the principal holding it (`seat.rs:73`). The arena keeps its own closed, hashed
`Channel` enum (`Helm`, `Sensors`, `Fire`; `Channel::ALL`) and an `EpochTable` keyed `(unit,
Channel)`. The two vocabularies meet by wire word: a cockpit station's `ChannelId` names are
`Channel::as_str`'s `helm`, `sensors` and `fire`, and M1a writes that mapping down as a table with a
test. A mind's functions are the stations' verbs, grouped by the arena `Channel` they need. A mind
holds **no F3 lease** and is never a lease holder (it is never a principal, D-UM-52): it acts on the
arena epoch table's channels of the stations its hull has, its seal names and no seated holder
occupies, under the seal's warrant principal. A call to a function whose channel the seal does not
grant is refused at landing with `Channel`, the word the resolver already has, and the refusal lands
in the notice buffer. The cockpit's combat verbs are what the helm, sensors and fire stations of a
combat hull offer; they are one hull's set, not the definition of a unit. A future station — mining,
cargo, building — adds its station's `ChannelId`, a new member of the arena's `Channel` (a reviewed
diff of the hash, the snapshot and `Channel::ALL`, §19), its human verb and its mind function in one
cut, so the set stays closed and a pilot can always do what a mind does.

**The doctrine that a machine can do nothing a pilot cannot is kept in both directions.** Each of
the three designs quietly widened the cockpit's verb set (a `Goal` intent; `STAND`, `CLOSE`, `OPEN`,
`TURN INTO` rule actions; a Command class that orders). The rule (D-UM-10): a mind has no verb a
pilot at that station cannot type, and every intent kind this document adds gets its station verb in
the same cut. `COCKPIT_VERBS` grows from 15 to 17: `DRIVE` (not `GOTO`; 17b cut M1a.4, §24) and `MIND` (its length tests at
`cockpit.rs:7934` and `:9276` are reviewed changes).

The mind's functions today, by channel:

- Helm: `helm(mdeg)`, `speed(cms)`, `startup()` → `Heading`, `Speed`, `Startup`, unchanged; and
  `goto(cell | spot)`, `hold()`, `goto_off()` → a new continuous `IntentKind::Goal(MotorGoal)` on
  `Channel::Helm`, the same intent the pilot's `DRIVE <spot|x,y>`, `DRIVE HOLD` and `DRIVE OFF` stage
  (the verb's word as built, 17b cut M1a.4, §24);
- Sensors: `lock(contact)`, `emcon(on)` → `Lock`, `Emcon`, unchanged;
- Fire (a hull with a weapons station only): `fire(weapon, slot)` → `Fire` carrying a condition
  citation (§5), and a mind never fires without one; `arm(weapon, target, min_permille, aspects,
  keeps, min_range_m, max_range_m)` and `disarm(weapon | ())` (the signatures as built, 17b cut
  M2.3, §24; first written `fire(slot, condition)`, `arm(slot, condition)`, `disarm(slot)`) → a
  new continuous `IntentKind::Condition` on `Channel::Fire`, the same intent the pilot's
  `FIRE <weapon> WHEN …` stages (§5), each arming and each citation checked against the sealed odds floor at its landing and refused `Unlawful` under it (§5; amended 2026-10-04 after review, finding 2);
- any channel: `trace(text)` → the trace ring only (§14).

`startup()` restarts a hull. The seal is taken at the principal's **dispatch**, the act by which a
character principal sends the hull out of its hangar or base: in the arena, `BOARD` for a pilot and
`Boarding → Running` for the AI fill; IC, whatever dispatch verb the corridor's design gives the
principal (who may dispatch an unseated hull IC is that design's question, and the answer is never
the mind). A mind cannot think before its seal exists, so it never takes itself out: its first
`startup()` executes the dispatch inside the sortie the seal has already opened (D-UM-45). In the
field a `startup()` after a power-down outside a base continues the same sortie under the same seal.

**A mind never orders (D-UM-11, amended by R-C).** A mind never issues `ORDER`, `RELEASE`, `PLACE`,
`BOARD` or `EXIT`; from `IntentSource::Mind` each is refused with `Channel`. The rule beneath it is
the Warrant's: no automated system may command, and only a **character principal** issues `ORDER`
(D-UM-52). A character may be human-played or AI-played; an NPC lance-leader is a person under the
Warrant and gives orders as a human pilot does; a unit mind is never a principal, whatever its class
or core. The engine therefore checks that an order's author is a character, never that it is "not a
mind". That check cannot live on `Order.issuer`: `issuer: Option<ObjId>` is the **unit whose
contact scope a target hint is written in** (`deliberation/order.rs:292-300`; the ordering pilot's
tank, or `None` for the squad's commander unit), `ordered_target` resolves the hint in that unit's
contact store, and D-UM-15 re-keys from it; a character is not a unit and has no contact store. So
`issuer` keeps that meaning, and `Order` gains a separate principal field, `by: CharacterRef` (an IC
character, or in the OOC sim a sim persona, below), a record field read by nothing that steers and,
like `spot`, not folded into `state_hash`. The principal check sits at the **world and bridge
admission**, where seats map to characters and where the deliberation bridge submits a leader's
order, not in `apply_intent`, which never sees principals: an order whose `by` names no character
entitled to order that squad is refused there. `apply_intent` keeps refusing `ORDER` from
`IntentSource::Mind` with `Channel`, the second wall. The economy angle's Command class is not
adopted. W7's deliberation layer is not "the machines' commander": it is an NPC lance-leader issuing
orders under their own warrant through this same path (D-UM-53, with the doc 16 amendment it needs).

**Where a lance-leader sits (D-UM-53).** An NPC lance-leader is an AI-played character and is never a
`Controller` value: there is no seat activation for it, and every hull of its squad, its own
included, stays `Controller::Mind`, flown by its sealed mind under its seal's warrant principal (D-UM-2: a
*seated pilot* means a seat activated by a player's connection). The leader **rides the squad's
commander unit**: `deliberation/squad.rs`'s commander, the lowest unit id in the squad, becomes the
leader's hull. Its hints are therefore written against its own hull's contact set, which is exactly
today's `issuer: None` scope, so hint resolution and D-UM-15's re-keying do not change; and when that
hull is wrecked the leader is down and the squad drops to doctrine, which is today's "a commander
that becomes a wreck does not hand over" rule read honestly. Cut L amends `squad.rs`'s "not a role,
not a capability, not a station" doctrine to say so: the commander is still chosen by lowest id and
still grants no station or capability, but it is where the leader rides, and the order's author is
the leader in `by`, never the unit. Cut L mints one leader per AI fill squad, so every fill squad
that deliberates today still deliberates and no behaviour changes; a squad with no character leader
— an IC squad dispatched without one — runs on doctrine. In the OOC sim each fill leader is a **sim
persona**: minted for the match, named only on that side, gone with it, with no IC character record,
no warrant and nothing IC touched (D-UM-50); an IC lance-leader is an IC NPC character with an IC
warrant.

**The motor (D-UM-3).** `reflex.rs` does two jobs. Deciding — the state machine, the utility score,
goal precedence, the withdraw and rally clocks, fire politeness — moves into the Rhai stock mind.
Driving — `follow` (`:3616`), `steer_aim`, `first_step_is_open`, `line_is_open`, `walk_is_open`,
`braking_speed_cms`, the avoid list, the path budget (`ReflexMemory::may_search`) and the
per-substep `hold_routeless` pre-pass (`:1247`) — stays native as `arena/motor.rs`, owned per unit
as a `MotorRow`, driven by `DueKind::MotorTick` (today's `ReflexTick` renamed and stripped of
`decide`), with every bound unchanged (`path_max_cells` 65,536, `repath_ms` 4,000,
`max_path_queries` 64 per window, `STEER_LOOKAHEAD_CELLS` 8, the wander rule). Path search in an
interpreter would cost about a thousand times its native price; this is why the motor does not move.
`ReflexCounters` becomes `MotorCounters`, which moves the harness baseline keys in the same cut. The
motor is the helm station's, so it serves every hull: a hauler's route home is the same bounded A\*
as a tank's approach.

The motor is the hull's drive-by-wire and the pilot's `DRIVE` drives it too (the word as built, 17b cut
M1a.4, §24): a pilot who types `DRIVE ridge` gets the same bounded A\* the machine gets, and `HEADING` or `SPEED` typed afterward
disengages it, as a mind's raw `helm` does. When a `Goal` lands, the motor runs one `follow` at the
landing millisecond, so a new goal takes effect at N+1 and does not wait for the next motor tick.

The motor's reads need an audit in M1a: its goal pickers (`far_entry_cell`, `entry_cell`,
`hold_goal`, `patrol_goal`, `stand_off_cell`) read terrain, zones, the waypoint graph, the unit's own
pose and its own plot, all public or own; its occupancy reads (`walk_is_open` and the index) are
*physical* — a hull bumps what is there and learns it as its own `blocked` event — and a mind sees
them only as that notice. Each kernel gets a test that holds the plot fixed while the truth moves.

**Landing (D-UM-12).** `Arena::order`'s body becomes `Arena::apply_machine_intents(unit, kinds,
t_ms, epochs)`, shared by the reflex during migration and by minds after it, with two changes: an
Emcon arm that calls `resolve_emcon` (`intents.rs:917`), which the present body lacks because a
machine never issues `EMCON`; and epochs taken from `MindState.think_epochs`, recorded when the job
was collected, instead of the current table. `authorises()` requires the exact current epoch
(`state.rs:3050`), so a seat reclaimed between think and landing refuses the stale thoughts with
`Refusal::Epoch` through the gate that exists (D-UM-13). Every other gate — wreck, channel,
`PowerNotOnline`, `Linkdead`, `Cycling`, `Empty`, `Overheated` — applies unchanged.

**`IntentSource::Mind` (D-UM-14)** joins `Seat` and `Machine`. It is distinct so the ledger, the
debrief and the panel attribute a mind's acts (`Mind fires cannon at BA (62%, side).`). It is not
counted on the per-unit seat window (`orders_in_window`), because the think cap of 16 and the pool
bound it already and a second hashed counter is not worth its snapshot row; a condition fire is
journalled `by: cond` and counted on neither.

**Orders reach minds (D-UM-15).** `apply_intent`'s two `controller != Reflex` gates
(`intents.rs:633, :676`) become "is a machine", the order is written to `MindState.order` instead of
`ReflexMemory::set_order`, and expiry stays sim-side on `StandingOrder::expired_at`. `issuer` stays
the hint-scope unit (the ordering pilot's tank, or the squad's commander unit, which cut L makes the
lance-leader's hull); the order's author is `by` (above). The target hint is **not** resolved as
`ordered_target` does today (`reflex.rs:3118` resolves it in the issuer unit's contact space to an
`ObjId`): it is re-keyed at write time through
`draw_contact_id(seed, unit, target)` and kept only if the unit itself holds that contact, else none.
`Objective::Support{of: ObjId}` is re-keyed the same way to the charge's `ContactId` or none, and the
Support branch's read of the charge's own reflex row or lock is dropped: a supporting mind follows
what it can see. The order vocabulary is today's combat objectives; a mining or hauling objective
joins it with its station, issued by a character principal like any other. The `Refusal::NotReflex`
wire string `not_reflex` is kept until `Controller::Reflex` is deleted (§17), when it becomes
`not_machine` in one reviewed cut.

---

## 5. The shot decision and the fire law (P1)

This section is the weapons station's. A hull without one — a miner, a hauler, a builder, an unarmed
scout — has no `Fire` channel, no condition slots, no odds in its view and no fire law, and nothing
in the rest of this document depends on it having them.

P1 is the centre for an armed hull. Real weapons will have multi-second cycles and precious
ammunition; there is no auto-fire button; what the engine supports is the *decision* — odds for any
contact, the target's aspect, ammunition salience — and the era's SOP is that the human drives the
conditions and the machine implements. The three angle designs put this in three places (native
slots; rules evaluated at think cadence; a resolver refusal). The synthesis is one mechanism with two
surfaces and one law.

**The mechanism (D-UM-16).** A **condition slot** is native engine state on an armed unit:
`FireCondition { weapon: slot, target: Lock | Contact(ContactId) | BestHostile, min_permille: u16,
aspects: mask over front/left/right/rear, min_rounds_left: u32, min_range_m: u32, max_range_m: u32 }`
(as built, 17b cut M1b.1: `min_range_m` so `range > n m` fits), up to the core's `condition_slots`
(`STOCK_CONDITION_SLOTS = 2` until M3), held per hull in `Arena.conditions` for a hull of any controller
(this document said `MindState.conditions`; a piloted hull has no mind row — §24), folded into the hash. A native pass `Arena::evaluate_conditions(unit, t_ms)` runs at the weapon's
own `DueKind::WeaponReady` (its no-op arm filled in 17b cut M1b.1), at every sensor pass or received
share that refreshes the lock's plot, at the startup flip, at every landing (once, after every intent of
it) and, for a seat, after a `LOCK` or an arming resolved in phase 4 (as built; §24). It reads the same per-contact odds
and aspect the view carries — the plot through `aim_at` and `resolve_seen`, never the target's live
state. When a condition holds it builds `IntentKind::Fire`, resolves it at that millisecond through
`apply_intent` (under `IntentSource::Cond`) and `resolve_fire_by` (as built), which charge the cycle and the ammunition as any shot does, and
the fired ledger line gains `by: cond` plus the slot. Evaluation is bounded by slots × events and
costs no mind ops: the hardware paid for the slot. A core's slots on a hull with no weapons station
are simply unused; the class table (§9) does not change by role.

**The slot and 16c's Q45.** The owner ruled on 16c `Q45` (2026-10-02): no armed trigger and nothing
that fires for the pilot; cadence parity comes the other way, the machines paying the seat's pulse,
which this document gives by construction. A condition slot is not Q45's armed trigger. That trigger
was a one-shot `FIRE` held until the weapon was ready, a fire-when-ready with no decision in it. A
slot is the principal's **shot decision written down**: which weapon, which target (a classified
one only), at what odds and aspect, keeping what reserve, never under the Compact floor. It fires
only when the plot meets every term, and it is announced each time it fires or holds. This is P1's
"the human drives the conditions and the machine implements", and the owner's own condition-triggered
fire. Parity now runs in both directions. The machines pay the seat's pulse: every mind-issued
intent, a cited `FIRE` included, thinks at N and lands at N+1 (§6), so no mind fires on its own tick
the way the reflex does today. And the seat and the mind reach the `WeaponReady` millisecond only
through a slot a principal armed, on equal terms from the same core; nothing fires that a principal
did not decide. 16c's Q45 row (status `unit minds, open`) is closed by D-UM-16 and D-UM-17; its status
is updated in 16c when M1b lands (§19), and Q-UM-16 asks the owner to confirm the reading.

**The law (D-UM-17).** Every mind-issued `FIRE` cites a slot: the Rhai `fire(slot, condition)` is a
one-shot citation evaluated at landing like any slot. A mind that emits a `FIRE` whose cited
condition does not hold on the plot is refused with a new `Refusal` word, `Unwarranted`
(`intents.rs:327`, `as_str` at `:402`; it reaches the wire and the cockpit sentence table). A
pilot's typed `FIRE` is the human trigger W8 asks for and is never gated. The **legal floor** binds every condition, however it is armed (amended 2026-10-04 after review,
finding 2): no lawful slot names an unclassified contact (`BestHostile` ranges only over classified
contacts; a `Contact(id)` or `Lock` target must be classified when evaluated, else the slot is
idle), and every lawful slot carries at least the Compact odds floor,
`[tacsim.minds].odds_floor_permille`, seed value 100. The floor is checked wherever a condition
enters the hull — on the card at the seal, and at the landing of every arming (a pilot's `FIRE
<weapon> WHEN …`, a mind's `arm`) and of every cited fire, ahead of the `Unwarranted` test — against
the floor the seal captured, so a policy change never touches a sealed sortie; before seals exist
(M1b) it is the match's configured floor, captured at check-out. A condition under the floor, or
naming an inadmissible target, is refused with a second new `Refusal` word, `Unlawful`, with the
floor named: refuse-not-clamp, for a card at the seal and for an arming alike, and the cockpit
pre-checks a typed rule off the frame so it is refused as typed. A rule with no odds term compiles
to the floor. Only an ungoverned core skips the check (D-UM-42), and its unlawful arming is
journalled. The player angle's `weapons:
free` mode does not survive as a mode: "free" is the labelled SOP `BestHostile` at the floor, which
the Line preset arms, and it is printed as a condition like any other.

**The surface (D-UM-18)** is the player angle's rule language, which compiles to slots:

`<ACTION> [<weapon>] WHEN <condition> [AND <condition>]…`, at most four conditions, no `OR` (two
rules are clearer than one), with the English explanation written back on save
(as built, 17b cut M1b.4, with no slot number: `Arming cannon: fires your lock when your chance to hit is
60% or better and it shows its side or rear.`). Conditions: `odds [weapon] >= n`, `aspect = front|side|rear` (a comma list,
identified contacts only), `range < | > n m`, `band = close|effective|far`, `target =
identified|moving|stopped|hurt`, `target <section> = green|yellow|red|destroyed` (an earned
reading), `rounds [weapon] < | > n | n%`, `reserve`, `own <section> < n%`, `hit within n s [from
front|side|rear]`, `hostiles >= n`, `heat > n%`, `order = …`, `lock`, `no lock`. Actions: `FIRE
<weapon>`, `HOLD FIRE <weapon|ALL>` (its unconditional form is spelled `FIRE <weapon|ALL> OFF` as built, 17b cut
M1b.4; the conditional veto is M5's), `LOCK nearest|best_odds|weakest|flank|newest` (not built in M1b; for a `LOCK`
rule the target conditions are tested on each candidate). The helm actions the player angle listed
(`STAND`, `CLOSE`, `OPEN`, `TURN INTO`, `HUNT`, `BREAK OFF`) are **not** rule actions: they are the
stock mind's posture sliders (§11) implemented through `goto` and `helm`, because a rule action must
be a station verb and those are not. A rule whose conditions fit a slot compiles to one; a rule that
needs a think (`hit within`, `hostiles >=`, `order =`) is evaluated by the stock mind at its think
and arms or disarms slots. The card says which is which (`R1 … (armed on the core)`,
`R2 … (checked each think)`). The rule language is the weapons station's in v1; a later station may
bring rule actions of its own under the same grammar, each a verb of that station.

Precedence fits in four lines and puts safety first: an `ORDER`'s `hold_fire` beats everything, and
it is now enforced for `IntentSource::Mind` and for condition fires in `resolve_fire_by` (17b cuts M1b.1 and
M1b.2: the `Unwarranted` answer after the `Cycling` and `Empty` tests and before `Overheated`; before M1b it
was honoured only in reflex planning, `reflex.rs:2862`; D-UM-19); a matching `HOLD
FIRE` beats any `FIRE` on the same weapon; then the first matching rule wins per channel; rules beat
sliders.

Pilots get the same slots from the same core: `FIRE cannon WHEN odds >= 60 AND aspect = side,rear`
typed in the cockpit stages `IntentKind::Condition`, `WEAPONSTATUS` shows the arming on its own
line under the weapon's row (`  S1: >=60% side/rear keeps 8`, as built in 17b cuts M1b.3 and M1b.4 for the
120-column floor: the bracket tail this document first drew reached 134 columns on a shipped row), a typed `FIRE` is
arbitrated by the weapon's cycle against a slot's fire (whichever lands first fires; the other reads
`Cycling`), and every slot shot is announced once (`Cannon fires at BA by S1 (62%, side).` — the slot's number, as built), with a
held shot said at most once per contact per 10 s (`S1 held the cannon: 54% under your 60%.`). The
armed escort of a mining lance gets exactly this: its pilot drives and decides the conditions, and
the core fires only when they hold (Q45 above).

---

## 6. Scheduling: a due event at the lattice millisecond, landing one substep later

The lead's pin is to think on pulse N and land at N+1. A per-pulse controller breaks D-16-2, and the
engine angle's "collect at substep N's end, before the hash" is wrong twice: substeps are cut from
each pulse's own start so a late wake ends off the lattice, and notices are staged after the hash.
The reflex's pattern is right, and this document copies it (D-UM-20). Everything here is stated in
the arena's terms because the arena is the only sim task that exists; a region map's sim task uses the
same due kinds, the same landing rule and the same pool, and names any divergence in §24.

**The think is a due event.** `DueKind::MindThink { unit, think_seq }` is scheduled every
`think_ms` of the core's class, at the lattice millisecond `stagger_ms(seed, unit, think_ms)`
(`rng.rs:251`, the sensor pass's and the reflex's own rule) plus `k × think_ms`. It resolves inside
whichever substep its timestamp falls in, ordered among that substep's due items by timestamp, then
kind, then unit id, after the sensor passes, impacts and `WeaponReady` of the same millisecond — including an impact a landing of that millisecond schedules at it: the millisecond's already-queued events, then every landing, then whatever those landings scheduled at that millisecond, then the thinks (as built, 17b cut M1b.0; §24) (amended 2026-10-04 after review, finding 3; M1a.5 as built ran the thinks before such an impact, §24). At
that instant the sim task builds the `ThinkJob { unit, think_seq, t_ms, view, memory bytes, seal,
epochs }` from the state as it then stands; the view's notices are the buffer as filled by every
post-hash scan up to the end of the previous substep (as built, 17b cut M1a.5: `stage_notice` fills the
buffer at staging time, so a think also reads a notice staged earlier in its own millisecond; §24). That
is the snapshot of N.

**The landing is a due event.** `DueKind::MindLand { unit, think_seq }` is scheduled at `t_ms +
substep_ms`, the sim task's substep length read when the think was collected (250 ms on shipped
arena content; `Clock::substep_ms` is a function of config, cell size and the field's top speed and
not of the tempo, which is why the cross-tempo fixture holds). It resolves in whichever substep its
timestamp falls in, before that substep's motion if it falls on the substep's first millisecond, in
unit-id order, each unit's intents in emission order, through `apply_machine_intents`. That is N+1:
one lattice step of sim time, never a tempo pulse, so the event order is the same function of sim
time at every tempo. The cross-tempo fixture extended with two mind units is a gate (D-UM-21).

**Execution policy (D-UM-22).** Jobs are pure functions of frozen inputs, so where and when they run
cannot change a result; only wall time moves. The sim task (`arena_task.rs` today, outside the
library, tokio allowed) owns an `Arc<dyn MindRunner>` into a process-wide pool of
`[tacsim.minds].workers` threads, default 2, shared by every sim task and served first-in first-out
per task batch. When a wake is one substep (the Engaged baseline: one substep per wake, four wakes per
arena second) the jobs collected in that step are sent to the pool after egress and awaited on a
oneshot before the next step: think time is off the wake's critical path and outside the wake lens.
When a landing's substep falls inside the same wake as its think (a Quiet pulse, catch-up, the
harness's jittered wakes) `Arena::step` calls the runner synchronously at the landing point, inside
`catch_unwind` and the stall watchdog, and the lane counts that time in the window. Backpressure: a
batch is bounded by the task's pool ceiling (§9), so the wait is bounded; a batch that is still
running when its task is due is awaited, never skipped, and the lane reports `pool_wait_us` so a
saturated pool is seen as late wakes, not as a different result.

**Snapshot and resume.** A snapshot taken between a think and its landing carries the think's
**answer**, not its job (amended 2026-10-04 after review, finding 1; D-UM-23): `MindState.pending`
is `{ think_seq, t_ms, epochs, intents, memory }` — the runner's result and the three channel epochs
recorded at the think — written at the think, hashed whole, carried by the snapshot and consumed at
the landing, and a resumed landing applies it through `apply_machine_intents` at the recorded epochs
exactly as the uninterrupted run would. Resume never re-runs a think: the view it read is gone by
the time a snapshot is written (its notices drained, its ranges settled, the rest of the substep's
events and the pulse's phases run), so a recompute cannot reproduce it. The sim task awaits any
outstanding think batch before it encodes a snapshot (M4's pool, D-UM-22). A fixture takes a
snapshot between N and N+1 — with a notice the think drained and an event after the think in the
same substep — and asserts the hash sequence equals the uninterrupted run's at three cadences
(D-UM-23). An IC sortie may run for hours across many snapshots; nothing here assumes the
short life of a match.

---

## 7. The sandbox and the budget

**The crates (D-UM-1).** `crates/tacsim/src/arena/mind.rs` holds `MindState`, the two due kinds,
job collection, landing, hashing and snapshot rows, and `trait MindRunner { fn think(&self, job:
&ThinkJob) -> ThinkResult; }`. It names no Rhai type, so `arena_library.rs` keeps passing and the
sim is tested with a native runner. `crates/mush-mind` is a game-neutral Rhai sandbox for unit minds
depending on `rhai` alone: the engine builder, op-and-fee accounting, canonical memory encoding and
hashing, the seal cache. `crates/tacsim/src/minds` holds the TacSim API on that engine (view types,
kernels, the station functions), the stock minds as content (`content/minds/standard.rhai` plus
manifest; later role minds beside it) and the `RhaiRunner`. The registry (§10, §16) is a separate
crate, `mush-roll`, with economy and org edges and **no** `mush-script` edge: conformance compiles
through `mush-mind`, never through the world engine. `mush-script`'s engine is deliberately not
reused: its `on_progress` samples a wall clock and its packages carry `timestamp()`.

**The engine (D-UM-24).** One shared `Engine` per process, rhai pinned **exactly** `=1.26.0` at the
workspace (which pins `mush-script` too, harmlessly), features `sync` and `serde`, so `Engine` and
`AST` are `Send + Sync` and the pool shares one engine and one AST per seal. Built with
`Engine::new_raw()` plus `LanguageCorePackage`, `ArithmeticPackage`, `LogicPackage`,
`BasicIteratorPackage`, `BasicArrayPackage`, `BasicMapPackage`, `BasicStringPackage`. Excluded:
`BasicTimePackage` (a wall clock), `BasicMathPackage` (transcendentals and parsing),
`MoreStringPackage`, `BasicFnPackage`. `DummyModuleResolver`; `eval`, `import`, `export`, `Fn`,
`call` and `curry` disabled through `disable_symbol` (the mechanism `mush-script` uses for `eval`;
it acts on keywords and literal tokens only and cannot touch an identifier, so the core package's
`sleep` is shadowed by an uncatchable refusal and its `exit` is a return — as built, 17b cut M2.1,
§24; a test asserts no time, eval or keyword-function symbol resolves). `on_print` and `on_debug` are set
(`new_raw` installs none) and write to the think's trace buffer: at most 8 lines of 120 characters,
never journalled.

Limits (D-UM-25, amended 2026-10-04 after review, finding 6): `set_max_call_levels(12)`,
`set_max_expr_depths(48, 24)`; the data-size limits as a shared backstop against runaway allocation
only — `set_max_map_size(256)`, `set_max_array_size(8_192)`, `set_max_string_size(8_192)` bytes,
twice the largest valid Class III memory, because Rhai 1.26 counts nested elements and string bytes
cumulatively over a whole value at every assignment (the limits as first written refused a Class III
`mem` past 64 keys, a third 32-int array in every class and a fifth 32-character string in every
class), the per-key, per-value and per-class limits of D-UM-30 living in `memory_check` and a test
asserting the engine limits are at least twice the largest class's totals from the class table;
`set_max_variables(64)`, `set_max_functions(32)`, `set_max_modules(0)`,
`set_strict_variables(true)` (a typo is a seal-time error), `set_fail_on_invalid_map_property(true)`,
`set_allow_anonymous_fn(false)`; loops allowed because ops bound them; `set_max_operations` at the
tier ceiling as a backstop only. Integer helpers: `abs`, `min`, `max` (the Arithmetic and Logic
packages' own, as built), the native `clamp` and `isqrt`,
`sin_m` and `cos_m` over `motion::sin_fx`'s table, `bearing_to(cell)`, `range_to(cell)`,
`arc(a, b)`.

**The budget (D-UM-26).** Thread-locals hold the think's op limit and a fee counter.
`on_progress(ops)` aborts with the cause word `ops` when `ops + FEE > limit` and never reads a
clock. Kernels add their fee when called and the trip lands at the next op. Rhai's operation count
is a deterministic function of the AST and the engine version, so the trip lands at the same point on
every host. Fees are calibrated once on the blessing host at about 42 ns per op (1/24 ops per µs):
`best_hostile` 4 + 3 per row; an odds or aspect read 1; `line_to(contact)` 20 and capped per think by
tier because it is a trace; each nav kernel 30; each `rand` 2; a fixed 20 per think for scope setup
(measured 0.99–1.34 µs at zero contacts). A station that joins later publishes its kernels' fees
with it.

**Parameters (D-UM-27).** The card is injected as a read-only custom type `card` — the second
argument of `fn think(view, card)` as built (17b cut M2.3, §24), nothing being in scope — not folded
into the AST: one AST per `(source_sha256, MIND_ABI)`, so 300 machines on 300 differently tuned cards
compile once (the engine angle's `optimize_ast` folding would compile once per card). Compile plus
seal measured 270 µs, paid at the seal in the world, never in the sim task.

**Floats (D-UM-28).** Rhai's float operators are built-ins and the workspace cannot apply `no_float`
to minds alone (features unify). Floats may appear in expressions; no float function is registered;
memory values and intent arguments are INT only and a FLOAT reaching either is a type fault (D-16-4
carried into the script). Cross-platform determinism of basic IEEE operations between the Windows
blessing host and the Linux production build is asserted by a CI test over the twin corpus (Q-UM-9
answered A).

**Randomness (D-UM-29).** A sixth `DrawLabel`, `Mind { unit, think_seq, n }`, `n` the call index
within the think; `rand(k)` returns `draw(seed, label) % k`. A label, not a stream: a mind's draws
never shift another draw. `DrawLabel::LABEL_KINDS` and the free `LABEL_KINDS` (`rng.rs:74, :174`)
and the five-example array in `arena_library.rs:92` change in one reviewed diff.

**Lints.** `mush-mind` carries `mush-script`'s deny list (`unwrap_used`, `indexing_slicing`, `panic`,
`unreachable`, `panic_in_result_fn`) and `expect_used` — all six, as built (17b cut M2.1, §24).
`crates/tacsim/src/minds` sits under tacsim's crate deny,
which omits `indexing_slicing`, so that module adds it at module level. `catch_unwind` across a Rhai
callback is the last backstop, with the stall watchdog.

---

## 8. Memory, faults and the drop to doctrine

**Memory (D-UM-30)** is `mem`, a Rhai map that persists between thinks for the whole sortie — bound
as `this` to `fn think(view, card)` as built (17b cut M2.1, §24), not a name in scope.
Keys at most 24 characters, the key count capped per tier; values INT, bool, unit, a string of at
most 32 characters, or an array of at most 32 INTs, checked when the think ends. Because `rhai::Map`
is a `BTreeMap` it encodes canonically; `state_hash` folds its FNV digest and the snapshot carries
the bytes. The sim task holds bytes, never a `rhai::Map`, so Rhai stays out of `arena/`. Memory
starts empty at the sortie's start (check-out, in the arena) and is the reflex's decision fields made
generic: `state`, `target`, `last_plot`, `unheld_since_ms`, `quiet_since_ms`, `rallied`,
`stand_until_ms` become stock-mind keys. Memory ends with the sortie; nothing a mind remembers
survives the power-down in a base.

**Faults (D-UM-31).** The op budget; a call-level, depth, size or variable limit; a runtime type
error; a memory-shape violation; an intent overflow; an unknown contact or slot. A faulted think
lands nothing and leaves memory unchanged; the sim task journals `EventKind::MindFault { unit,
think_seq, cause }` and increments `MindState.strikes`; a clean think resets them. Three consecutive
faults drop the unit to doctrine, the `HALTED` rule of `docs/help/scripting.md` applied to a hull. (A
call to a station function the seal does not grant is a refusal, not a fault, §4.)

**Doctrine (D-UM-32).** *Drops to doctrine* is defined once and names one thing: the hull's **drive
ROM**, Class 0 (§9), running the Registry's doctrine for the hull's role at the hull's fallback
preset. On every hull that exists today that is **Standard Doctrine** (the stock mind, §11) at its
fallback preset (Line unless the owner set another); on a hull with no weapons station its fire half
is absent, so it drives, senses, holds and returns on its orders (D-UM-48, Q-UM-12); when a role
doctrine joins the Doctrine class with its station, the ROM of a hull in that role runs it. The ROM is
not a component and cannot be destroyed; it holds the helm, so a tripped or core-killed hull can still
break off or make for base. The drop happens on the third strike, on `destroy_core` (§9), and at the
seal when a licence has lapsed (§10); it lasts for the rest of the sortie, and is journalled
`EventKind::MindDropped` with a cause word. The pilot, owner and warrant principal read one line
(`Your core tripped: over its 1,500-op budget three thinks running. The hull is on the ROM; MIND is
off until you power down at a base.`; in the arena, `MIND is off for the match.`); nobody else does.
The deliberation layer's "mind link dropped" sentence (`arena/ledger.rs:407`) is retired with W7's
reframing (D-UM-53): an NPC lance-leader's order that expires unrenewed reads as that leader's orders
lapsing, in the words a human leader's would, so the two vocabularies do not collide. The ROM mind
cannot fault: its ceiling is set at twice its measured worst case on the twin corpus, armed and
unarmed hulls both, and that is asserted.

---

## 9. Hardware: the core, the classes, the pool (P5)

**The component (D-UM-33).** `core` is a new content kind, the ninth in `definitions/mod.rs KINDS`
and the seventh in `CONTENT_KINDS`, authored under `content/cores/` and installed as
`ComponentRole::Core { core }` beside `Weapon` and `Sensor` (`definitions/unit.rs:383`, lowered near
`:1386-1640`). It mints a capability `tacsim.core` with a minted rules schema the way `Power` does
(the only role that mints one today), carrying class, `think_ms`, ops, rows, `odds_rows`, slots,
memory keys, `line_to` cap, profiles and the governed bit. Check-out decodes it into
`UnitState.core: CoreProfile` as it decodes `SensorProfile`. A hull with no core component is Class
0: every hull has the ROM, armed or not. Its `InstalledComponent.asset` is a titled object, so IC a
core is its own economic asset, separable from the hull. `odds_rows` and slots are fire-control
figures; on a hull with no weapons station they are carried and unused.

**Placement and damage (D-UM-34).** Content places the core in a section: on the sim's chassis the
turret on the line tanks (fire control belongs there, and losing the turret loses the guns and the
box in one line), the chassis on the drone and the bipeds, the hull on the heavy; an unarmed hull
carries it wherever its content says. Two closed-vocabulary consequences join D-16-9's list in both
`Consequence` enums (`definitions/unit.rs:281`, `resolution.rs:153`): `degrade_core` (one class lower
until repaired at a base; in the arena, for the rest of the match) and `destroy_core` (the mind drops
to the ROM at its next landing for the rest of the sortie; continuous orders hold; a human aboard
keeps manual control). Neither the M32 revocation path (`wreck_ops` revokes only power, mobility and
weapons) nor `remove_weapons` (weapons only) does this today. Fitting a core IC is a refit work order;
`UnitMutationOp` has no install op, so M7 adds `UnitMutationOp::InstallComponent` under the M32
plan/apply/journal discipline (Q-UM-10 answered A).

**The classes (D-UM-35).** The three angle designs disagreed by three orders of magnitude; only the
engine angle's numbers fit the measured interpreter and the lens, and they are adopted. Think periods
are given at the 250 ms substep. The classes do not vary by role: a miner's line core is a tank's.

| Class | Name | Thinks every | Ops per think | Contact rows | Odds rows | Slots | Memory keys | Profiles | `line_to` |
|---|---|---|---|---|---|---|---|---|---|
| 0 | the drive ROM | 1,000 ms | 400 | 8 | 0 (the lock only) | 1 | 16 | 1 | 1 |
| I | the doctrine core | 1,000 ms | 1,500 | 12 | 2 | 2 | 32 | 2 | 2 |
| II | the line core | 500 ms | 3,000 | 16 | 4 | 4 | 64 | 3 | 4 |
| III | the command core | 250 ms | 4,500 | 24 | 8 | 6 | 128 | 4 | 6 |

Ceilings per unit per sim second: 400, 1,500, 6,000 and 18,000 ops, about 17, 63, 250 and 750 µs of
CPU at 24 ops/µs. Class 0 runs only the Registry's doctrine, at any slider values. The names are lore
(Q-UM-1 answered: names ok): compute is scarce, so automation is excellent and people are decisive.
The *command core* is named for the command vehicles that carry it, not for commanding: no core of
any class orders (D-UM-52). The sim chassis declare their cores in content: the line_tank family and
both bipeds II, heavy_mk1 III, scout_mk1 and drone_mk1 I (a scout is flown by its pilot), target_mk1
the ROM alone. Hulls for other roles declare theirs when they are authored.

**The pool (D-UM-36).** Each sim task has a deterministic compute allotment, the engine's guarantee
that no purchase can break the lens: `ops_per_task_s = max(120,000, 500,000 − 1,000 × units)`,
200,000 at 300 units and 476,000 at 24. Admission counts class ceilings, never measured use.
Recounted for the harness's **300** machines: all at Class 0 take 120,000 and leave 80,000, about four
Class III upgrades at 17,600 net each; a 12v12 of Class III takes 432,000 and fits. In the arena the
lobby splits the pool evenly between sides at match setup and first-come within a side; `SIM MIND`
and `SIM UNIT` check at choice time and whichever comes second is refused with the arithmetic (`This
side would think 96,000 ops a second; its half of the centre's pool is 100,000 and 92,000 are spoken
for.`), so `BOARD` never fails because of a core; check-out refuses an over-subscribed roster by name
as the last gate. On a region map admission is at the startup in the hangar or base, refused there
with the same arithmetic before the hull leaves, so a sortie never fails mid-way because of its core
and a hull already out is never evicted; how a region's pool is split (by owner, by organisation, by
traffic authority) is the corridor's design. The pool is visible in the fiction (Q-UM-2 answered A):
the centre's compute is finite, `SIM MIND` prints the arithmetic, and IC an allotment is a resource
an operator sells or rations, so it can be fought over.

**The sim (D-UM-37).** In the OOC simulator a pod emulates the chassis's content-declared core and
the lobby lead may set the AI fill's class (Q-UM-3 answered A). The OOC sim never reads a character's
owned IC hardware: hulls and cores in the sim are the pod's. Its minds are the one IC fact it reads,
read-only, under D-UM-49. Owned hardware matters only on IC sorties (which do not exist yet). The
economy angle's parity rule is not adopted: the proving ground compares minds on the chassis they
will fly, and a lead who wants parity sets it.

---

## 10. The licence and the governor (P5, P6)

**Classes of mind (D-UM-38).** Three legal classes, enforced by construction: a class's engine
instance registers only that class's host functions, so a script that calls outside its class fails
at the seal. *Doctrine*: the Registry's published standing procedures, free, perpetual, open,
loadable on any core; at launch Standard Doctrine alone, the combat role's; role doctrines for
mining, hauling and building join it with their stations. *Discretion*: filed minds with bounded
discretion over their own hull's station channels, the verb set of §4; licensed by term.
*Frontier*: never mountable in a hull and never the author of an order or any act; in the fiction it
advises human offices and nothing more (D-UM-53). There is no Command class (D-UM-11, D-UM-52).

**The licence (D-UM-39)** is a `mush-roll` record: holder (character or org), charter and line, pin
(a revision, or follow), seats (concurrent sealed sorties), term on the `WorldIc` clock (at most 90
IC days, the Compact's audit term; nothing but Doctrine is perpetual), the warrant principal, form
and state (Active, Grace up to 3 IC days, Lapsed, Revoked by the Registry only, Recalled when the
revision failed). Renewal is a fresh auto-drafted E4 contract per term, so every licence payment
takes one path and one split. **A lapse never touches a sealed sortie**: the seal carries the state at
the sortie's start, a hull already out finishes its trip under it, and at the next seal — the next
startup leaving a base — the hull drops to doctrine and only its own principals read why. A seat is
taken at the seal and freed at the sortie's end; the OOC sim takes none (D-UM-49).

**Transferability (D-UM-40).** The owner's P6 is "traded or copied between players: yes definitely",
so licences are **transferable** instruments: a holder may sell on a licence they no longer want, the
transfer moves the entitlement and the seller loses it, and an author may mark a listing
non-transferable. `InstrumentKind::Entitlement`'s doc comment says non-transferable and nothing
enforces it; the comment is amended to "non-transferable unless the catalogue entry says
otherwise", and a mind licence is a catalogue entry that says otherwise. Sales and transfers between
characters of one account are refused through the lobby's `AccountId` oracle (alt laundering). A sale of a licence with no sortie out settles at once. A sale while sorties it seals are out puts
the licence in **PendingTransfer** (amended 2026-10-04 after review, finding 7): at most one
transfer is pending per licence and a second sale is refused by name; no new seal admits the licence
for either party while it is pending, dispatch admission refusing it by name (`Sentinel 2.1 is
changing hands; no new sortie may seal it until the transfer settles.`); every sortie already sealed
runs to its close under the seal it carries, the seller its warrant principal, the seal never
touched; no renewal is drafted for the seller meanwhile; settlement is one atomic bundle — the
holder changes, the escrowed contract pays its one balanced split, the pending record clears — when
the last admitted sortie closes, or at the deadline, the earlier of the term's end and
`[tacsim.minds].transfer_settle_ic_days` (default 3, the Grace length) after the sale; sorties still
out at settlement keep the seats they hold until they close, so open sealed sorties never exceed
seats, the buyer starting with seats minus those held, and each such hull drops to doctrine at its
next seal exactly as under a lapse (D-UM-39), told only to its own principals; a transfer that
settles at the term's end delivers the licence in Grace with its renewal drafted to the buyer; a
Registry revocation cancels a pending transfer and refunds the escrow, a recall travels with the
buyer told; the seller and buyer are told when the transfer goes pending (open sorties, deadline),
the seller at each close, both at settlement, and nobody else. The deadline and the held-seats rule
are defaults pending the owner (Q-UM-18). Charters trade too (§16).

**The governor (D-UM-41).** The governed bit in the core's capability payload. A governed core
refuses at the seal: an unfiled digest, a line with no live licence, a class beyond the licence, any
slot outside the lawful vocabulary or under the odds floor. The sim task never reads a licence; it
trusts the seal as it trusts every other check-out fact. In the OOC sim every pod's core is governed,
and its governor checks the character's sim set (D-UM-49) in place of a seat. The sim set includes the
character's own unfiled drafts, so the sim governor admits them (a pilot may seal `mine r7` before it
is ever filed, as the proving ground does); every other rule — the lawful-slot vocabulary, the odds
floor, the class against the pod's core — applies there exactly as IC. Licence payment, seats and
terms do not, because nothing IC is spent.

**Contraband (D-UM-42, built last).** Ungoverning a core is a refit at a facility without a
`core_integration` charter (the Tether's back room at Kirkwood); the work order exists in the
industry journal and the core's provenance records it. An ungoverned core seals unfiled drafts
(no fee, no levy, no Roll entry), runs above its licensed class, runs lines nobody licensed to its
holder, and on an armed hull arms unlawful slots: fire on unclassified contacts, which beats W5's
classification delay, or under the floor — the **unwarranted act**, the setting's central crime
(Q-UM-6 answered A). Heirlooms (pre-Retreat Enclave dies: Class III performance, no governor) and
greys (Martian dies off an unchartered line: no lineage) are its hardware. Detection is evidence
only, through five channels: renewal attestation (an ungoverned core cannot attest), custody
inspection (deterministic, reads the bit and the provenance), the black box (W16: every IC sortie
opens with each unit's seal line), testimony (a hit by a contact the victim's own sensors never
classified), and an author's recognition of their own mind's quirks in a hull they never licensed to
its holder (D-UM-55). Consequences: seizure as a `Disputed` encumbrance then title to the Registry's
principal, a fine as an E4 obligation, revocation of the principal's licences for a term, Tycho
voiding cover — the everyday deterrent that needs no police. No contraband cut ships before its
detection channel exists, and none before IC sorties exist.

**The quirk (D-UM-55).** A mind behaves like itself: an author who sees a corporation's miners
pause the way theirs do on a lost contact, or an escort break off at their odd threshold, may suspect
their code is running unlicensed. The game gives no oracle and no match score: recognition rests only
on what the player's own sensors earned in the field (the observer rule), and the engine says nothing
to confirm or deny it. The author's recourse is a claim to the Registry, which is testimony that
opens a custody inspection or a black-box read of the suspected hulls' seal lines; the inspection, not
the suspicion, establishes the fact (a seal's source hash against the author's filed revisions, or an
unfiled digest on an ungoverned core). The claim is the evidence channel; the recognition is play.

---

## 11. The card: sliders, presets, profiles (P2)

The card is the parameter block a mind reads, declared by the mind as a schema from a closed widget
vocabulary — `slider(min, max, step, default, unit, help)`, `choice([...], default, help)`,
`toggle(default, help)`, `bearing(default, help)`, a facet set, a contact-class set — and rendered
by every client from that schema (D-UM-43). A value outside its range is refused with the range
named, never clamped. The vocabulary is role-neutral; cards, presets and profiles are per role
(D-UM-48). The stock schema below is Standard Doctrine's, the combat role's and the only one that
exists; it maps onto today's `ReflexConfig` (`config.rs:425-447`) so that Standard Doctrine's
**Line** preset on its defaults is today's machine — the regression anchor. A card section that
drives a station the hull lacks (fire discipline on an unarmed hull) is shown inert with the reason
(`no weapons station on this hull`) and never refuses the seal. A mining or hauling stock mind brings
its own schema and presets (a shift preset, a convoy preset) with its station.

*Posture.* `stance` ∈ hold, close, orbit, hunt, screen (default hold; the bot's words plus the
`ORDER` objective's). `band_pct` 30–100 step 5, default 70 → `engage_band_permille` 700 (the card
prints the metres for the chassis: `420 m on line_tank`). `close_m` 100–400 step 50, default 200.
`orbit_sense` cw, ccw, auto. `turn_into_hits` on/off, default on, `turn_s` 2–10, default 4 (the
bot's rule). `screen_bearing` 0–359, `screen_arc` 60–180, default 120. `opening` wait, search,
advance, default wait (the bot's muster opening).

*Fire discipline* (a weapons station only) — the P1 levers, each a slot the stock mind arms.
`min_odds` 10–95 step 5, default 10, which is the Compact floor and today's behaviour (the reflex
fires whenever ready with line); `aspect` any, flank, rear, default any; `reserve_pct` 0–50 step 5,
default 0 (today keeps nothing back); `reserve_odds` 50–95, default 75; `ident` identified only —
there is no `any` setting, because the floor forbids it.

*Targeting.* `prefer` nearest, exposed, hurt, balanced, default balanced, each a labelled setting of
`weight_range`, `weight_facet`, `weight_health` whose defaults are 100, 40, 30 (there is no odds term
in today's score; `best_odds` is a new choice that weighs the odds column and is **not** the
anchor). `switch_m` 50–300, default 100 (keep the lock unless another hostile is that much nearer).

*Survival.* `break_off_pct` 10–80, default 35 → `withdraw_permille` 350. `fight_home_pct` 0–100,
default 17.5 → `stand_min_permille` 175 (the slider steps in halves). `rally_s` 0–120, default 20
→ `rally_ms` 20,000; 0 never rallies.

*Search.* `search_s` 0–120, default 60 → `search_ms` 60,000. `hunt_after_s` 0–300, default 90 →
`hunt_after_ms` 90,000. `stop_unseen` on/off, default on.

**Presets (D-UM-44)** are named cards and nothing more; a preset never carries a value a slider
cannot show. Standard Doctrine's six combat presets: *Line* (the defaults above), *Sentry* (screen,
min_odds 55, opening wait, hunt_after 0, rally 0), *Sniper* (hold, band 100, min_odds 65, reserve 30,
stop_unseen on), *Skirmisher* (orbit, band 85, aspect flank, min_odds 35), *Hunter* (hunt, band 60,
search 60, hunt_after 45, min_odds 30), *Rearguard* (hold, break_off 70, rally 0, min_odds 50).
Picking a preset copies it into the profile; the card then says `Line, edited (min_odds 60)` until it
matches one again. Other roles' presets arrive with their stock minds.

**Profiles.** A mind carries one to four named profiles by core class (§9), all sealed together. A
pilot switches between sealed profiles with `MIND PROFILE <name>`, a recorded intent at most once per
10 s: switching is not an edit, so P4 and replay hold. On a long IC sortie profiles are how a mind
changes posture between legs (a hauler's run and its approach to base) without breaking the seal.

---

## 12. The flow: choosing, sealing at the hangar exit, the sortie, releasing

**The sortie (D-UM-45).** A sortie is one unit's trip from the startup that takes it out of a hangar
or base to the power-down that ends in a hangar or base, the same one or another. The seal is per
sortie. A power-down in the field (a stop to wait out a storm, a dead-engine drift, a hull left on a
claim) does not end it: the next `startup()` or `STARTUP` continues it under the same seal. A sortie
also ends when the hull is destroyed, or when it is recovered into a hangar or base by other means
(towed, carried); it never ends because a pilot left or a licence lapsed, and an engagement, or a
match on an IC map, never ends it. In the OOC arena the match's end is the pod's power-down: when
custody returns to the pod at the match's end, that is the hangar return that closes every unit's
sortie, and the match ledger journals `SortieClosed` there (§13). What the
unit does on the sortie — fight, mine, haul, build, nothing — is no part of the seal's definition. A
lance is one sortie per unit: an armed escort piloted by a human and three AI miners each carry their
own seal, taken as each leaves.

**In the OOC sim lobby.** `SIM MIND` beside `SIM UNIT` in the style of `match_/sim.rs`'s selectors,
during selection only. Bare, it prints what the slot will carry and what of the character's IC
holdings fits the chassis's core (`Yours: none. Fits line_tank (II): Standard Doctrine, Sentinel 2.1
(licensed), mine r7.`) — the minds the character holds IC and nothing else (D-UM-49). `SIM MIND mine
[PROFILE Sniper] [@r7]` sets it; with no pin the latest saved revision is resolved at each `SIM
START` and printed then. `SIM AI MIND Sentinel PROFILE Sniper` is the side lead's: it sets the mind
the side's AI fill runs, from the lead's own holdings, default Standard Doctrine on Line, which is
today's machine; under Q-UM-15's default, where the other side has a human pilot, the fill and any
released hull fly Doctrine only.
Fit (class, rules, profiles, the pool) is checked when the choice is made; the later of `SIM UNIT`
and `SIM MIND` is the one refused, so `BOARD` never fails because of a mind. While selection is open
only the slot's own character reads which mind the slot will carry, and for the AI fill only the side
lead who set it; everyone else reads `a mind` against a slot that carries one, or nothing. Once the
match starts nobody's mind is printed on the board, and no mind, listing, licence or author is named
to anyone else in the sim, before or after (D-UM-49).

**Sealing (P4, refined by R-B).** The seal is taken at the hangar exit, before the startup that
begins the sortie. In the arena the pod is the hangar and `BOARD` then `STARTUP` is its exit: nothing
a seal holds can change between them (selection is closed), so the seal is computed at the dispatch
(§4): at check-out on `BOARD` for a pilot, and at `Boarding → Running` for the AI fill, whose mind's
first `startup()` then executes that dispatch inside the sealed sortie. On an IC region map it is
computed at the principal's dispatch in the hangar or base, ahead of the `STARTUP` there. Either way the
pin, the whole card (every profile and its rules), the core, the licence class and state, the warrant
principal and the station channel set are frozen into the seal (§13). The station view says so in
one line before `STARTUP`. Editing a mind in another window during the sortie makes a new revision
and changes nothing aboard until the hull powers down in a base and starts up again.

**What a mind holds in v1, and the T-54 boundary.** The three angle designs all assumed a pilot and a
mind could split the hull's channels (ASSIST: the pilot drives, the mind locks and fires). That needs
per-channel K6 leases (16c T-54, unbuilt) and a `(unit, channel)` epoch the mind owns. V1 therefore
offers two modes only: **NONE** (the sealed mind is dormant) and the **whole-unit mind**, which acts
on every channel its hull's stations grant (helm, sensors and fire on a combat hull; helm and sensors
on an unarmed one), holding no lease (§4), and flies AI fill, released hulls, proving matches and,
IC, the unseated miners and haulers of a lance. P1's co-piloting is still delivered to a seated
pilot of an armed hull in v1 — not through the mind but through the core's condition slots, which are native and evaluated for a
piloted unit exactly as for a machine (§5): the pilot drives, the core fires under the pilot's armed
slots. `ASSIST` and `MIND ON|OFF <channel>` arrive with the cut that gives a unit a second seat
(§22, after M8; Q-UM-11 answered A).

**Release and possess.** A pilot who goes linkdead keeps their continuous orders and hands nothing
over (today's rule); once `Released` past the grace (or on `EXIT`, or a revoked lease) the hull goes
to its sealed mind (to Doctrine instead in an OOC match with a human on the other side, under
Q-UM-15's default), and the board reads `flown by its mind`. `BOARD` takes it back as today; a
re-seat is a seat activation inside the same sortie, never a new seal. Possess and act-as are the
existing seat path: `mush_sim::request_seat_activation`, then `apply_control` raises the `(unit,
channel)` epochs and calls `set_controller(Pilot)`, which refuses in-flight thoughts through
`Refusal::Epoch`. New behaviour: `set_controller(Pilot)` does **not** delete the `MindState` row (the
reflex's `release` deletes its row today); the mind's memory, conditions and notices stay dormant,
and a release adopts the mind again with its memory intact. A pilot who boards a hull already out on
a region map takes it under the seal it carries.

**The `MIND` verb** (the seventeenth cockpit selector; `WATCH` is the fifteenth and `DRIVE` the sixteenth
since 17b cut M1a.4, §24; on a future station's
view the same verb). Bare: a status block of at most seven lines under 120 columns — mind and seal,
core and class, profile, who acts on each channel, ops at the last think against the budget, each
rule's truth at that think, the last decision. `MIND PROFILE <name>`; `MIND RULES`; `MIND TRACE [n]`
(the last n thinks, default 3, one line each plus intents). On the TUI and the web cockpit screen a
MIND block of six rows and at most 40 columns joins the right-hand column under the weapons block (or
in its place on an unarmed hull); the map keeps its 78 columns; the first three rules carry a state
glyph; clicking a rule on the web prefills `MIND PROFILE …` and sends nothing until Enter. The card
draws the same way everywhere, one slider a line with a 10-cell bar and the unit-aware value; a rule
shows its English under it.

---

## 13. Records, the seal and replay

**The seal** is computed world-side at the sortie's start (§12), in order: the hull's loadout and
stations; the licence state; the core's class, governed bit and condition; the revision (a follow pin
resolves to the newest revision filed before the seal); the card validated against the schema,
refused not clamped. It holds: unit and sortie id, core component, class, governed bit; charter,
line, revision; `source_sha256`; the canonical card bytes and their digest; the odds floor it was validated against (2026-10-04 after review, finding 2); the armed slots (none on
an unarmed hull); `MIND_ABI` (`rhai=1.26.0;api=N;stock=M`); the licence id or `doctrine` (in the OOC
sim, the holding it reads, D-UM-49); the warrant principal; the station channel set (as arena
`Channel` members, §4); the fallback reason if any. It lives on `CustodyUnit` beside `sensor`,
`weapons` and `gunnery`, for every unit, piloted or not, armed or not.

**Where it goes (D-UM-46).** A seal record, `MindSealRecord`, is keyed by `(unit, sortie)`. The
**sortie** is the unit of record; the match is a window onto it. In the arena the `MatchRecord`
(`record.rs:334`) gains `minds: BTreeMap<ObjId, MindSealRecord>`, each the seal of the sortie the
match lies within (in the arena exactly one per unit: it opens at the pod's exit and closes at the
match's end), and the match ledger opens with one `EventKind::MindSealed` line per unit (unit,
sortie, mind id, the first 16 hex of the source hash, card digest, class, governed bit, licence
validity, warrant principal), so the ledger digest attests which minds flew, and closes with one
`EventKind::SortieClosed` line per unit when custody returns to the pod at the match's end (or
earlier at the unit's destruction, with the word). On an IC region map each unit's black box (W16)
opens each sortie with its `MindSealed` line and closes it with a `SortieClosed` line at the
power-down in a base (or at destruction or recovery, with the word); any engagement, haul or shift on the way is recorded inside
that pair and cites the sortie. Sources live immutably at `tacsim:mindsrc:<sha256>`. `state_hash`
(`hash.rs:485`) folds the `MindState` rows after the reflex rows (`:593`) in unit-id order: seal
digest, `think_seq`, `pending_think_seq` and `pending_t_ms`, `think_epochs`, memory digest, the range
map, the notice buffer, order, strikes, dropped, the condition slots, the `MotorRow`; an
`each_new_row_field_moves_the_state_hash` test as `reflex.rs:5911` does. The snapshot (format 5 since 17b
cut M1a.5, §24) carries the rows whole and the seals by reference. In the arena a resume refuses closed when
`MIND_ABI` or the rhai version differs, the precedent being `SnapshotError::Physics`, and D-16-3
voids the match by name; a match is minutes long and OOC, so that loses little. An IC sortie may be
out for hours, and voiding it across a server upgrade would strand a hull in the field, so the IC
rule differs: a resume across a `MIND_ABI` or rhai change keeps the sortie, its seal and its memory
bytes, drops every mind hull on it to the drive ROM (the Registry's doctrine as the running server ships
it, which needs no old engine), and journals `MindDropped` with the cause word `abi`; the sealed record
keeps the old ABI, so the black box shows which engine thought before the break, and the hull's next
seal at a base is on the new one. Operators are still told to prefer an upgrade window with few
sorties out; whether the region sim task can do better (keep the old engine for open seals) is
recorded in §24 when that task is built.

**Replay** here means re-simulation, not cut I's picture. The check-out snapshot, a **seat-intent
tape** and the seals reproduce the per-substep hash sequence. M6 persists the tape
(`tacsim:tape:<match>`) for OOC matches, under its own bound `[tacsim.minds].tape_max_bytes` and the
centre's `HISTORY_DEPTH` of 20, so the bench can step any think of any kept match. The player angle's
per-unit picture tape is not kept: at 300 units it is a storage bomb, and the seat tape plus
determinism regenerates any picture on demand. A replay across an engine version is refused in words
(`Match 31 was thought on mind engine 1; this server runs 2.`). What an IC sortie's replay keeps is
the corridor's design; the seal and determinism make it possible, and nothing here sizes it.

---

## 14. The REPL, the proving ground and the bench (P7)

**The trace ring.** Per mind unit, in sim RAM, outside the hash like `last_frames`: the last 64
thinks — `think_seq`, `t_ms`, ops, fee, the landed intents with their outcomes, the rules with their
conditions' values (`R1 FIRE cannon: odds 54 >= 60 false; aspect side true -> no`), each kernel
called and its fee, the author's `trace()` lines, any fault. It egresses on a new `arena.mind` feed
(and a region task's equivalent) at most once per sim second, only to the unit's principal (the
seal's licensee, owner and warrant principal), the side lead for their own AI fill, and staff. A
mind's trace says what its platform saw, so nobody else may read it: the observer rule applied to the
debugger. For a mind held under a sealed licence, the trace shows its rules, card, intents, ops and
the author's `trace()` lines, never `mem` or source.

**The proving ground** is the bot's lanes made a verb: `@mind/prove mine/line VS Standard/Hunter 30
MUSTER 1v1`. It is part of the OOC sim and obeys D-UM-49: the opponent is any Doctrine preset, any
mind the character holds IC, or `mirror`, and nothing else. Sparring against another author's
published mind is an IC service between characters and is not offered in the OOC proving ground
(D-UM-50); it is a natural offering of the Ceres Simulation Center (D-UM-51). Scenarios DUEL, MUSTER
(the figure of record), HOLD are the combat role's; the scenario set is content, so a role's
proving scenarios (a haul under threat, a mining shift) join with its station. Sizes 1v1 to 6v6. N is
10–100 and even: each of N/2 seeds flown once from each side. Proving matches are headless arenas,
deterministic, no wall clock, run unthrottled on `[tacsim.minds].proving_workers` (default 1, one run
per account at a time), never sharing a worker with a live match; a harness lane asserts two running
proving arenas leave the live p99 where it was. The quota is unit-minutes per account per day
(default 2,000). The report reuses the bot's lines in the game's voice: won, lost, drew, timed out as
its own column; `win rate 19 of 30: 95% Wilson 46 to 78%`; `lost the first section in 11; won 2 of
those`; shots by weapon and odds bucket; rules fired and held; hits by facet; rounds left; ops p50 and
p99; faults; the seeds. Two minds on the same seeds print `difference called` only when the intervals
do not overlap. `wilson_95` moves from `mush-bot/tests/support/arena.rs:1439` into a tacsim library
module outside `arena/` (it is f64).

**The bench** is a booth in the simulator centre with one verb, `BENCH`, rendered on screen 0 with
the station view and the MIND block. A **closed-loop** session (`BENCH LOAD 14/7`) re-runs proving
match 7 of run 14 from its seed under a manual clock, which needs no tape because no human was in it:
`STEP [n]`, `RUN TO 1:30`, `BACK [n]` (re-simulate from the start; cheap and exact), the hash shown at
every step and matched against the run's. An **open-loop** session (`BENCH LOAD MATCH 31 RA`)
re-simulates a kept OOC match from its seat tape to a think and answers "what would this mind decide
given this picture", never "what would have happened". In either: `PIC`, `TRACE`, `WATCH <expr>` and
`EVAL <expr>` (read-only over `pic`, `card` and `mem`, in a sandbox that emits no intent, on a
separate small budget), `DIFF <mind>` (a second mind the character holds, over the same pictures,
listing the thinks where intents differ), `SWAP <mind>` (forks the timeline and says so), `POSSESS`
and `RELEASE` (your station verbs drive the unit under the manual clock), `ACT AS <callsign>` (any
unit of a private run, the opponent's included). This is the NPC-REPL possess-and-act-as idea on a
deterministic substrate. Nothing in the bench touches a live match or an IC sortie. Rhai's own
`debugging` feature (statement stepping) is deferred until its overhead on the live interpreter is
measured, because features unify across the workspace.

---

## 15. The author's workflow

A library belongs to a character (minds are IC goods): their own minds and drafts, the lines they
hold licences for, and Doctrine. That library, read-only, is also the character's whole set in the
OOC sim (D-UM-49). One verb, `@mind`, carries the workshop: `@mind/new <name> [FROM Standard|
<mind>]`, `/list`, `/info`, `/source <name>[@rev] [from-to]`, `/paste` (lines until a lone `.`),
`/line`, `/insert`, `/delete`, `/check` (compile; run on three canned pictures — nothing in sight,
one hostile at 450 m, hurt with two hostiles, each on an armed and an unarmed hull; report line and
column, ops per think, intents, class violations, station functions the target hull would refuse),
`/save <note>`, `/card`, `/set <name>/<profile> key=value`, `/preset`, `/profile add|del|rename`,
`/rule <name>/<profile> n=<text>`, `/fork` (open minds only, lineage recorded), `/diff`, `/prove`,
`/publish` (§16). Revisions are immutable and content-addressed: `DefinitionDigest::for_canonical_value`
under domain `tacsim.mind.v1` over the normalised source, the schema, the presets, the class and
`MIND_ABI`; its first six hex digits are the seal id players see. Named versions are kept while you
own the mind; the last 50 unnamed revisions, plus any a kept record, run, sortie seal or listing
references. Sources are capped at 32 KiB, a library at 20 minds.

Three editors, each first-class for a different author. The web workshop is a Svelte page: the
library, a CodeMirror 6 editor with Rhai highlighting and inline compile errors from a `mind.check`
request, the card rendered from the schema, the rules with their English, a Try panel with the
canned-picture trace, Prove and Bench buttons. Over SSH, `mind push|pull|check` through a narrow
exec allow-list with public-key auth only, which amends design 02 §5.6's "interactive shell only"
(`mush-net-ssh/src/server.rs:1550`; Q-UM-7 answered A). The TUI edits cards and rules and views
source read-only.

The raw tier: one script with a header `const MIND = #{ name: "Sentinel", api: 1, class:
"discretion", role: "combat", core: "II" };`, an optional `fn card()` returning its schema, and
`fn think(pic, card, mem)`. The `role` names the stock mind and preset family the card extends and
the stations the mind expects; a mind may be written to run on any hull and test `pic` for the
stations it has. The stock library for the combat role is native kernels with published fees:
`stock::think`, `stock::rules`, `stock::fire_discipline`, `stock::best_hostile`, `stock::hold_band`,
`stock::steer_to`, `stock::hunt`. An author's first real mind is usually three lines calling
`stock::think` and overriding one decision. `help minds` (pilots), `help mindcraft` (authors; the
API, the stations, the fees, the classes, the seal and the sortie), `help proving`. Two tutorial hint
points join the five in `tutorial.rs` (`HintPoint`): after the first finished match (`SIM MIND
Standard lets your tank's core run the Registry's doctrine by your rules`), and on the first
`@mind/save` (`@mind/prove <mind> VS Standard flies it thirty times and tells you how sure to be`). A
new character's library holds Standard Doctrine, open, with teaching comments and the six presets;
their slot carries no mind until they choose one; the fill they fight is Standard on Line.

---

## 16. The Roll and the market (P6)

**The market is IC only (D-UM-50).** Every purchase, sale, licence, levy and royalty in this section is
an IC transaction between characters or organisations, on the IC clock and the IC ledger. The OOC sim
neither buys nor sells, takes no seat, runs down no term and posts nothing IC (its own match ledger
is OOC, D-16-3); what a character holds IC
is what they have in the sim, at no cost (D-UM-49). An author's buyers are IC holders, and a mind's
IC worth is what it does on IC sorties (and, later, in the Ceres Simulation Center, D-UM-51).

**Filing.** Publishing a mind files a revision on the Registry's Roll of Minds — the Registry being
already the notary of last resort (doc 11 §2). Filing runs conformance (compiles under its class's
engine; the schema validates; at most 8 presets; the slots are lawful; the stations it calls exist
in the vocabulary; the reference bench rates the revision's worst observed ops per think on the
Registry's fixed scenario set for its role, so a revision that overruns a Class I core is rated "II
and above"), charges a fee to `system:tacsim.registry`, and refuses a normalised duplicate of another
charter's revision in one sentence that names nothing. Behaviour is not protected; a clean-room mind
that plays the same way is legal, and that competition is the market. The Roll entry is public (name,
house through the hidden-org renderer, class, role, lines, rated profile, forms offered); the
**charter** is a world object titled through `issue_title` by the Registry's principal. Selling the
mind is a title transfer; revenue follows title; a charter can carry a `Lien` or serve as
`Collateral`; stealing the object conveys nothing (doc 08 D5). A line is a compatible schema;
revisions within a line are free updates; followers seal the newest revision filed before their
sortie's seal, pinned holders keep theirs; an author may withdraw a revision, only the Registry
recalls one; a rhai bump re-runs conformance for every live revision and recalls failures with
notice.

**Private filing and the owner's licence (proposed 2026-10-04 after review, finding 5; D-UM-56 and
D-UM-57 pending the owner, Q-UM-17).** Filing and listing are separate acts. `@mind/file
<mind>@<rev>` files a revision exactly as `/publish` does — the same conformance, the same fee to
`system:tacsim.registry`, a charter issued to the filer, a public Roll entry whose forms offered
read `none` — and `/publish` files the revision if it is not yet filed, then lists it. Filing is not
release: D-UM-54 still admits NPC procurement only to a listed line, so an author who files but
never lists keeps the mind off every organisation's hull. Filing issues the charter's owner an
**owner's licence** to its line: no price, so no settlement, levy or venue share; the Compact's
audit term (at most 90 IC days) renewed by attestation like any other licence, so renewal
attestation (D-UM-42) reaches it; non-transferable; its seats bounded only by the hulls the holder
dispatches; following the charter's title, so when the charter changes hands the owner's licence
moves to the new owner as D-UM-40 moves a sold one and the former owner keeps no use. An unfiled
draft runs only in the OOC sim and the proving ground (D-UM-41) or on an ungoverned core, which is
contraband (D-UM-42); there is no IC exemption for one's own drafts. A derivative's filing runs the
ordinary conformance plus a lineage check — the parent licence live and held by the filer, the chain
depth at most 3, the royalty cap held — and its owner's licence is **dependent**: live only while
the holder's licence to the parent line is live, its term never outrunning the parent's; the
parent's author is paid for private use through that parent licence and for resale through the
parent royalty. When the parent licence lapses, is sold, is revoked or its revision is recalled, the
dependent owner's licence lapses at the same moment: the fork becomes source-sealed (its last filed
revision still runs and seals, no new revision may be filed) until the holder holds the parent line
again — the default; the alternatives, deleting the fork or letting it persist unlicensed, are
Q-UM-17's. Charters are refused between characters of one account as licences are.

**Listing and sale.** A filed line is a `mush-market` listing of `Representation::Entitlement`
under catalogue ruleset `minds`, reached through tacsim-registered `CatalogSource`,
`AvailabilityProvider` and `DeliveryProvider` tables; `mush-roll` takes no market edge, and the
public projection gate holds. `@mind/publish Sentinel@2 price=400 terms=sealed seats=6`. Terms:
*sealed* (the default: buyers run and tune it, read its card, presets, rated profile, notes and
probe-level traces, never the source), *open* (source and fork rights: a fork is a derivative draft recording its parent charter, line and revision, filed privately only while the holder's licence to the parent line is Active or in Grace, its owner's licence dependent on the parent's — proposed D-UM-57, 2026-10-04 after review; the parent's royalty, capped at 300 per mille with a chain depth of 3, is paid inside every derivative's settlement), *open-resale* (as *open*, and a filed derivative may also be listed; the listing shows `derived from Sentinel by Ada`). A purchase drafts an ordinary E4 contract with escrow; settlement is
one balanced `PayoutShare` split: the Compact levy (seed 50 per mille) to the Registry's sink, the
venue fee when a vendor org lists the line, the parent royalty, the remainder to the charter owner's
account. The `DeliveryProvider` writes the licence; payment never edits a loadout. A buyer follows
the major version unless they pin; a new major is a new listing.

**The vending machine is one seller among players by construction.** NPC houses write minds (staff
authored), file them on the Roll under their NPC org exactly as a player does, pay the fee and the
levy, and sell through a `mush-market` `Vendor` with budgets. The launch catalogue: the Stickney
Yards Doctrine Library (Standard Doctrine's preset pages, free; role doctrines join it); the
Meridiani Field Series (Discretion, sealed, the mercenary standard); Tycho Mutual's Safe Conduct
(convoy defence and the haul under escort; Tycho discounts cover on hulls that run it); the
Territorial Force's Coprates Drill (Republic-standing buyers only, through an
`AvailabilityProvider`).

**NPC procurement (D-UM-54, Q-UM-8 answered A with R-D's condition).** Later, NPC procurement buys
from the Roll on a monthly budget by the Registry's trial rating per credit, with a standing filter
and a diversity cap of 40 percent of an org's hulls per charter. It buys **only what an author has
released through a listing** — sealed, open or open-resale — and holds a licence like any buyer, its
seats counting its concurrent sealed sorties; there is no other road from a player's library to an
NPC hull. This is how a player who only writes AI sells to factions, and what passes the zero-PC
test: one player's code may fly the Republic's escorts, or a corporation's miners, against or beside
other players. A mind found running on an organisation's hulls without such a licence is contraband,
learned through evidence only (§10, D-UM-55).

**Hardware IC.** Dies reach the corridor as `tacsim.electronics` lots graded `die.c1|c2|c3` with
`Lineage::Origin` from a chartered importer (Basin Charter Instruments: a Luna depot, agents at Ceres
and Phobos, an allotment per IC month, which makes Class III scarcity a world fact). Stickney
integrates them (`core_integrate@1`); fitting is a refit work order. Prices relative to a line tank's
catalogue baseline: a Class I replacement 3 percent, II 8 percent, III 25 percent and
`Legality::Restricted`, sold only to a principal holding a `tacsim.licence.core3_end_user`
entitlement, checked at transaction time the way `extraction_authorized` checks an extraction
licence. The OOC sim never reads owned hardware (D-UM-37) and never posts to the IC economy ledger
(D-UM-50).

---

## 17. Migrating `reflex.rs` into Standard Doctrine, and the twin fixture

**What moves.** The state machine and its edges (`health_permille` under threshold → Withdraw;
`is_dry` → Withdraw for good; the rally once after `rally_ms`); the utility score in
`best_hostile_where`; Patrol's goal precedence; Approach to `stand_off_cell` at `stand_off_want`;
Engage (lock, fire when ready with ammunition, power and no overheat, gated by line); the Screen and
Support orders with `SUPPORT_LEASH_CELLS` and `SUPPORT_STATION_CELLS`; standing-order obedience and
expiry. What the port needs is exactly §3 to §5: the view with aspect, odds, `reads_dead` and the
order; memory for the clocks and the state; the kernels `best_hostile(w_range, w_facet, w_health,
prefer)`, the nav pickers, `line_to(contact)`; the station functions with `goto`, `hold` and `arm`.

**The unarmed hull (D-UM-48).** Today's reflex already runs hulls that cannot fight: the harness's
non-fighting units, and the range target, which is inert to it (doc 16 §6). Standard Doctrine is
written so that on a hull with no weapons station its fire half is absent, not idled: it starts up,
follows orders and the motor, searches, holds, withdraws and returns, and never calls a fire function.
The twin corpus includes unarmed hulls, and Gate A covers them. This is also the ROM's behaviour on
any hull before its role doctrine exists (Q-UM-12).

**The named divergences.** Four reads in today's reflex are past the observer rule and an
observer-pure port cannot copy them byte for byte: the `ObjId` tie-break in `best_hostile_where`
(→ `ContactId`); `reflex_has_line` tracing with `fire_line` to the target's true pose (→ `line_to`
traced to the plot cell); `ordered_target`'s Support branch reading the charge's own row or lock (→
the charge's `ContactId`, held or none); `contact_reads_dead` reading `target.fatal_sections` (→ the
own-kill notice or the held fatal reading). Two scheduling changes also apply: thinks at the lattice
millisecond instead of a staggered `ReflexTick`, and landing one substep later, which adds at most
250 ms of order latency; condition slots offset that for fire. All six are written into doc 16 §11
as the as-built note of the cut that flips the default. Today's machine is not reproduced by putting
a leak back.

**Three columns, two gates (D-UM-3 continued).** T0 is the shipped native reflex. T1 is
`NativeStockMind`, a Rust implementation of Standard Doctrine over `MindView` and memory, through the
mind schedule and the motor; it is the oracle for the Rhai port and the measured native fast path P3
allows. T2 is the Rhai stock mind. *Gate A* is exact: T1 ≡ T2 on every think's intent list and
memory bytes over a corpus of (view, memory) pairs harvested from 30 seeded authored-cohort runs,
armed and unarmed hulls both; on the cross-tempo fixture with two mind units (one event list, one
hash sequence at 5,000, 1,000 and 500 ms); and on whole-match per-substep hash sequences. *Gate B*
bounds the divergence: T0 against T2 over 30 pooled matches with sides swapped; the 95 percent Wilson
interval on T2's win rate must contain 50 percent; shots per minute, time to first shot, the state
distribution (`ReflexMemory::distribution`), path queries and corner refusals reported against T0. If
B fails the Line sliders are retuned until it passes before the default flips. After the flip
`Controller::Reflex`, `ReflexTick` and the decision half of `reflex.rs` are deleted and its motor half
lives on as `motor.rs`; `NotReflex` becomes `NotMachine` then.

---

## 18. Performance and the P3 gate

**Measured.** 19.5–24.1 ops/µs on mind-shaped code on the blessing host; 15.05 µs per think with
custom types at 12 contacts; 3.67–3.93 µs flat from 4 to 48 contacts when scoring is a kernel; 270 µs
compile plus seal, once per `(source, ABI)`. Those drive two choices: custom-type views, and a stock
mind written over kernels so its cost is flat in the number of contacts.

**Expected, recounted for 300 machines.** 300 Class 0 units at a 1,000 ms think over a 250 ms
substep is 75 thinks per substep and 150 per 500 ms window; the stock mind on kernels is estimated
at 150–250 ops including fees, 6–10 µs, so 0.9–1.5 ms of CPU per window, of which the native
`decide` it replaces was already inside the baseline. Unmeasured and reported separately: `view_us`,
the cost of `mind_view` for 75 units per substep including `aim_at` plus `resolve_seen` per odds row
(each a fire trace; an unarmed hull pays none); `odds_rows` per class is the lever if it is large. The
pool's ceiling at 300 units, 200,000 ops per sim second, is 100,000 per window: **4.17 ms** at 24
ops/µs. The engine angle's 4 ms CPU lens would therefore fail its own ceiling lane by construction;
the lens is set at **5 ms per window on both cohorts**, `[tacsim.minds].cpu_lens_ms`, separate from
the window lens (think time is off the critical path at Engaged) and inclusive of whatever ran
inline. The headroom against the window lenses stands at the baseline: CI window p99 10.56 ms against
12, authored 14.1–16.3 ms (18 an upper estimate) against 20, wakes 6.55 and 7.7–8.2 ms against 12.
dev1's ops/µs is unmeasured and may be markedly lower than ALIEN18's; the op budget is deterministic
regardless, the lens is wall time on whatever host runs it, and the pool is a config knob. These
figures are the arena's; a region map's sim task, when it exists, is measured by the same lane with
its own cohort before it carries minds.

**The lane.** `tacsim_harness_arena_minds` on the existing ignore string, the cohort checked out
through `tacsim::match_::custody_unit` as `Controller::Mind`, three variants on the production
composition: `minds_stock_ci` (the CI cohort, every machine on Standard at Class 0, its non-fighting
units as unarmed hulls), `minds_stock_authored` (the authored cohort, the same),
`minds_ceiling_authored` (the pool filled to its admission ceiling by a synthetic Class III load mind
that spends exactly its budget and emits a legal intent set). The lane runs the task's real policy —
wakes at `next_due_ms`, the batch on a 2-worker pool awaited before the next wake — with
`TACSIM_WAKE_JITTER_MS` forcing multi-substep wakes on a seeded fraction so the inline path is
measured. Under `TACSIM_ASSERT` and through `scripts/harness-weekly.sh` it asserts: wake p99 ≤ 12 ms
on both cohorts; window p99 ≤ the cohort's lens counting inline think time; mind CPU per window p99 ≤
5 ms; zero faults on the stock variants; ops per substep, peak and total, within the pool. Its
deterministic block (thinks, ops, fees, faults, landings, condition fires, the final substep hash)
joins the delta test against the committed baseline. Reported, not asserted: `view_us`, measured
ops/µs, think µs p50 and p99, `pool_wait_us`.

**The fast path has one trigger.** If `minds_stock_authored` misses a lens on the blessing host
after the custom-type view and the kernels are in, T1 is promoted to production for Standard
Doctrine at any slider values; its seal record says `engine native-stock`, and the Rhai stock mind
stays the exact reference it is held to by Gate A. Raw-Rhai minds are always interpreted. No other
fast path is admitted; a role doctrine that later needs one asks for it by the same trigger.

---

## 19. Fit with the crates

`crates/tacsim/src/arena/{mind.rs, motor.rs, view.rs}` (library, no Rhai, no tokio);
`crates/tacsim/src/minds/` (the API, the station functions, kernels, `RhaiRunner`,
`content/minds/standard.rhai`); `crates/mush-mind` (the sandbox; depends on `rhai` alone);
`crates/mush-roll` (drafts, the Roll, charters, licences, loadouts, seal records; depends on
`mush-core`, `mush-org`, `mush-economy`, `mush-mind` for conformance; no `mush-script`, no
`mush-market`, no `tacsim` edge; the market reaches it through tacsim-registered tables). Against 08i
§4 rule 1: drafts, Roll entries and charter metadata are bounded whole-document workflow records
(exempt, strict decoding, deterministic keys); licences, loadouts and seals decide which code runs
and which verbs a hull may emit — action legality — so they are authoritative typed families on the
prepared path from birth. One new Entitlement instrument, `tacsim.licence.core3_end_user`; one system
principal, `system:tacsim.registry`. The proposal's `system:tacsim.sim` is dropped: the OOC sim owns,
pays and is paid nothing IC (D-UM-50), and a sim seal's principal is the character who chose the
mind — the pilot, or the side lead for the fill — as an access and display fact only, never an IC
liability. A `FactProvider` publishes `minds.licence.lapsed`, `minds.licence.transferred`, `minds.filed`, `minds.seized`,
`minds.unwarranted`, `minds.claimed` (an author's quirk claim filed, D-UM-55) so contracts and fronts
can condition on them. `[tacsim.minds]` holds the classes, the pool formula, `cpu_lens_ms`, `workers`,
`proving_workers`, the quota, the odds floor, the levy, the fees, the term cap, `tape_max_bytes`.

Reviewed diffs this document knowingly makes: `LABEL_KINDS` 5 → 6; `COCKPIT_VERBS` 15 → 17;
`FRAME_VERSION` 1 → 2 with the bot parser and the mush-ansi goldens; `Refusal` gains `Unwarranted` and `Unlawful`;
`not_reflex` → `not_machine` at the deletion cut; `ReflexCounters` → `MotorCounters` moves the
baseline keys; rhai pinned exactly at the workspace; `InstrumentKind::Entitlement`'s doc comment;
`EventKind` gains `MindSealed`, `SortieClosed`, `MindFault` and `MindDropped`; the arena `Channel`
gains a member per new station, each a diff of `state_hash`, the snapshot and `Channel::ALL`
(D-UM-47), with the `Channel`-to-`ChannelId` mapping table and its test written in M1a; `Order`
gains `by: CharacterRef`, checked at the world and bridge admission (D-UM-52); `squad.rs`'s
commander doctrine, amended in cut L (D-UM-53).

Pending amendments to other documents, made when the named cut lands and not before: doc 16's W7
row and §6 Deliberation and Provider-failure paragraphs, with an as-built §11 entry, in cut L
(D-UM-53); doc 16's D-16-3 row gains the no-IC-crossing clause of D-UM-50 ("an OOC match posts
nothing to, consumes nothing of and reads nothing from the IC world but D-UM-49's mind holdings")
in M5, the first cut that reads an IC holding from the sim; 16c's `Q45` row is marked closed by
D-UM-16 and D-UM-17 in M1b, the cut that builds the slots.

---

## 20. Decisions (owner, 2026-10-02, binding)

D-UM-1 to D-UM-44 are the proposal's, kept in number and amended where the rulings reach them (marked
*amended by R-x*); D-UM-45 to D-UM-55 are new with the rulings. No proposal decision was split or
dropped.

| # | Decision |
|---|---|
| D-UM-1 | A mind is a pure function `mind(view, memory, card, keyed draws) -> (≤16 intents, memory')` run through `trait MindRunner` in `arena/mind.rs`, which names no Rhai type; the interpreter lives in `crates/mush-mind` (rhai only) and `crates/tacsim/src/minds`; the registry is `crates/mush-roll` with no `mush-script` edge. |
| D-UM-2 | `Controller` gains `Mind` (hash code 2); every unit without a seated pilot is `Controller::Mind`; `Controller::Reflex` and `ReflexTick` are deleted after the twin gates pass. (*Amended by R-C*:) a *seated pilot* is a seat activated by a player's connection; an AI-played character such as an NPC lance-leader is never a `Controller` value, and the hull it rides stays `Controller::Mind` (D-UM-53). |
| D-UM-3 | (*Amended by R-B*) `reflex.rs` is split: the decision half becomes Standard Doctrine in Rhai; the driving half stays native as `arena/motor.rs` on `DueKind::MotorTick` with every path bound unchanged, serving every hull's helm. The twin fixture has three columns (T0 shipped reflex, T1 native oracle, T2 Rhai); Gate A is T1 ≡ T2 exactly, armed and unarmed hulls both; Gate B bounds T0 vs T2 by a 30-match pooled Wilson interval containing 50 percent; leaks are never restored. |
| D-UM-4 | The parity rule (*amended by R-B*): a mind reads exactly what the unit's stations show a human at them — the `arena.cockpit` keyframe for a combat hull, that keyframe without the weapons block for an unarmed one, a future station's view when it lands — built by `Arena::mind_view` from the frame builders; a tier truncates and never extends; every mind field is first a `Frame` and keyframe field with a `FRAME_VERSION` bump. The cockpit row formatter moves into `arena/view.rs`. (*Amended 2026-10-04 after review, finding 4*:) a truncation is a declared tier's row cap, never the silent omission of a field another field indexes; `MindView` carries the keyframe's `scan_tables`. |
| D-UM-5 | (*Amended by R-B*) Aspect (`facet_band_seen` off the plot, on every hull), per-weapon odds for classified contacts only (weapons station only; `resolve_seen` against a new `Arena::aim_at`, limited to the core's `odds_rows`), `reads_dead` derived from the own kill or the held fatal reading (weapons station only; never `contact_reads_dead`), the standing order (re-keyed to the unit's own `ContactId`) and `t_ms` join the frame and the keyframe for pilots and minds alike. No course estimates. |
| D-UM-6 | Contacts are named in the API by the opaque `ContactId`, with designator and alias as fields. |
| D-UM-7 | The view's Rhai types are registered custom types with getters, never maps (15.05 vs 24.69 µs). |
| D-UM-8 | `MindState.notices` is a bounded (16), hashed, snapshotted buffer filled by the post-hash journal scan and drained per think; the mind's own refusals are appended to it. |
| D-UM-9 | A think is atomic: at most 16 intents into a native buffer; a fault lands nothing and keeps memory. |
| D-UM-10 | The verb set stays closed in both directions (*amended by R-B*): a mind has no verb a pilot at that station cannot type, and every added intent gets its station verb in the same cut. `COCKPIT_VERBS` grows to 16 with `GOTO` and `MIND`. `IntentKind::Goal(MotorGoal)` on Helm drives the native motor for pilots and minds; `IntentKind::Condition` on Fire arms a slot for both, on a hull with a weapons station. |
| D-UM-11 | A mind never issues `ORDER`, `RELEASE`, `PLACE`, `BOARD` or `EXIT`; from `IntentSource::Mind` each is refused with `Channel`. No Command class. (*Amended by R-C*:) the rule beneath it is D-UM-52's — only a character principal orders — and W7's deliberation is an NPC lance-leader's `ORDER` under D-UM-53, not an unchanged machine path. |
| D-UM-12 | Landing is `Arena::apply_machine_intents`, the factored body of `Arena::order`, with an Emcon arm calling `resolve_emcon` added, resolving at the landing millisecond through `apply_intent` and the existing resolvers. |
| D-UM-13 | Landed intents carry the epochs recorded at collection; a reclaimed seat refuses stale thoughts with `Refusal::Epoch` through the exact-epoch gate. |
| D-UM-14 | `IntentSource::Mind` joins `Seat` and `Machine` for attribution; it is not counted on the per-unit seat window; condition fires are journalled `by: cond`. |
| D-UM-15 | (*Amended by R-B and R-C*) Orders reach `Mind` units through the unchanged `Order` path; `Order.issuer` stays the hint-scope unit (the ordering pilot's tank, or `None` for the squad's commander unit, which cut L makes the lance-leader's hull), and the order's author is the separate `by` (D-UM-52); the hint, resolved in the issuer unit's scope, and `Support{of}` are re-keyed at write time through `draw_contact_id` to the unit's own held `ContactId` or none; the Support branch's read of the charge's row is dropped. Non-combat objectives join the order vocabulary with their stations, issued by a character principal like any other. |
| D-UM-16 | Condition slots (`FireCondition`: weapon, target Lock / Contact / BestHostile, `min_permille`, aspect mask, `min_rounds_left`, `max_range_m`), up to the core's slots, hashed, evaluated natively at `WeaponReady`, at a sensor pass refreshing the target's plot and at landing, firing through `apply_intent` and `resolve_fire` at no mind cost; pilots get the same slots from the same core via `FIRE <weapon> WHEN …`. (*Amended by R-B*:) slots are a weapons-station feature; a hull without one has none and needs none. A slot is the principal's written shot decision, not 16c Q45's armed trigger (which stays unbuilt); parity runs both ways, every mind intent landing at N+1 and seat and mind reaching the `WeaponReady` millisecond only through a principal's slot; this closes 16c's Q45 row (§5, Q-UM-16). |
| D-UM-17 | The fire law: every mind-issued `FIRE` cites a slot (`fire(slot, condition)`, evaluated at landing); a citation that does not hold is refused `Unwarranted`, a new `Refusal` word. The legal floor (*amended by R-B, and 2026-10-04 after review, finding 2*) — no unclassified target, `Lock` included; at least `odds_floor_permille`, seed 100 — is validated on the card at the seal and at the landing of every arming and cited fire against the floor the seal captured (before seals, the match's configured floor), refused with a second new `Refusal` word, `Unlawful`, refuse-not-clamp; a rule with no odds term compiles to the floor; an ungoverned core alone skips it (D-UM-42). There is no free mode; "weapons free" is the labelled `BestHostile`-at-the-floor SOP. A pilot's typed `FIRE` is never gated. |
| D-UM-18 | The rule language `<ACTION> [weapon] WHEN cond [AND cond]…` (≤4 conditions, no OR, English written back) is the weapons station's authoring surface in v1; rules that fit a slot compile to one, the rest are evaluated by the stock mind at its think; rule actions are `FIRE`, `HOLD FIRE`, `LOCK …` only, each a station verb. Precedence: an `ORDER`'s hold-fire, then `HOLD FIRE` rules, then first match per channel, then sliders. |
| D-UM-19 | An `ORDER`'s `hold_fire` is enforced in `resolve_fire` for `IntentSource::Mind` and condition fires (new resolver code). |
| D-UM-20 | (*Amended by R-B*) A think is `DueKind::MindThink` at a lattice millisecond (`stagger_ms(seed, unit, think_ms)` plus multiples), resolved in the substep its timestamp falls in after that millisecond's other due items, including any a landing of that millisecond schedules at it (*amended 2026-10-04 after review, finding 3*: landings, then the heap's events at that millisecond, then the thinks); the view is the state then, with the notices scanned up to the previous substep's end. Never a pulse. A region map's sim task uses the same rule. |
| D-UM-21 | (*Amended by R-B*) The landing is `DueKind::MindLand` at `t_ms + substep_ms` (the sim task's substep as read at collection), in unit-id order then emission order. The cross-tempo fixture with two mind units is a gate. |
| D-UM-22 | (*Amended by R-B*) Thinks run on a process-wide pool (`[tacsim.minds].workers`, default 2) shared by every sim task; at a single-substep wake the batch runs after egress and is awaited before the next step; when the landing falls in the same wake the runner is called synchronously at the landing point inside the watchdog. A batch is bounded by the pool ceiling and never skipped. |
| D-UM-23 | (*Amended 2026-10-04 after review, finding 1*:) a snapshot carries the think's **answer**, not its job: `MindState.pending` is `{ think_seq, t_ms, epochs, intents, memory }`, written at the think, hashed whole, carried by the snapshot and consumed at the landing at the recorded epochs; resume never re-runs a think; the sim task awaits an outstanding batch before encoding; a fixture snapshots between N and N+1 with a drained notice and a post-think event and asserts the hash sequence at three cadences. (*Amended by R-B*:) across a `MIND_ABI` or rhai change the arena's resume refuses closed and the match is voided by name; an IC sortie is kept, each mind hull on it dropping to the drive ROM with `MindDropped` cause `abi` journalled, the seal keeping the old ABI (§13). |
| D-UM-24 | The sandbox is one shared `Engine::new_raw()` with Core, Arithmetic, Logic, Iterator, Array, Map and BasicString packages only; no time, no math; `eval`, `import`, `export`, `Fn`, `call`, `curry` disabled; strict variables; `DummyModuleResolver`; `on_print` and `on_debug` to the trace ring. rhai pinned exactly `=1.26.0` at the workspace; `MIND_ABI = rhai + api + stock versions`, recorded per seal; a rhai upgrade is a reviewed ABI bump. |
| D-UM-25 | (*Amended 2026-10-04 after review, finding 6*) Limits: call levels 12, expr depths 48/24, variables 64, functions 32, modules 0, anonymous functions off; the data-size limits (map 256, array 8,192, string 8,192 bytes — twice the largest valid Class III memory, asserted against the class table) a backstop against runaway allocation only, since Rhai counts nested elements and string bytes cumulatively; the per-key, per-value and per-class memory limits are D-UM-30's, enforced by `memory_check` at think end; `max_operations` at the class ceiling as a backstop only. |
| D-UM-26 | The budget is ops plus kernel fees checked in `on_progress` with no clock anywhere; fees calibrated on the blessing host at about 42 ns per op; every think pays a fixed 20. |
| D-UM-27 | The card is injected as a read-only custom type in scope; one AST per `(source_sha256, MIND_ABI)`, cached. |
| D-UM-28 | Floats may appear in expressions; no float function is registered; memory values and intent arguments are INT only (a FLOAT is a type fault); a Windows-vs-Linux hash-equality CI test over the twin corpus is mandatory. |
| D-UM-29 | Mind randomness is a sixth `DrawLabel`, `Mind { unit, think_seq, n }`; no stream, no other entropy. |
| D-UM-30 | (*Amended by R-B*) Memory is a bounded canonical map (per-class key cap, keys ≤24 chars, values INT/bool/unit/ASCII string ≤32 bytes/array ≤32 INTs — ASCII since 2026-10-04 after review, finding 6, so `memory_check`'s count and Rhai's byte count agree) living for the sortie; its digest is folded into `state_hash`, its bytes carried by the snapshot; the sim task holds bytes, never a `rhai::Map`. |
| D-UM-31 | (*Amended by R-B*) A fault journals `MindFault` with a cause word; three consecutive faults drop the unit to doctrine. A station function the seal does not grant is refused `Channel`, not a fault. |
| D-UM-32 | (*Amended by R-B*) "Drops to doctrine" means the hull's Class 0 drive ROM running the Registry's doctrine for the hull's role — Standard Doctrine at the hull's fallback preset on every hull today, its fire half absent on an unarmed one — for the rest of the sortie; it holds the helm, cannot be destroyed and cannot fault; journalled `MindDropped`. (*Amended by R-C*:) the deliberation's "mind link dropped" sentence is retired with D-UM-53; a lance-leader's lapsed order reads as that leader's orders lapsing. |
| D-UM-33 | (*Amended by R-B*) `core` is a new content kind and `ComponentRole::Core`, minting capability `tacsim.core` with a minted rules schema as `Power` does, decoded at check-out into `UnitState.core: CoreProfile`; a hull without one is Class 0, armed or not; the component's asset is a titled object. |
| D-UM-34 | (*Amended by R-B*) Content places the core in a section (on the sim chassis: turret on the line tanks, chassis on drone and bipeds, hull on the heavy). Two new closed consequences, `degrade_core` (one class lower until repaired at a base; in the arena, for the match) and `destroy_core` (the ROM for the rest of the sortie), in both `Consequence` enums. IC fitting is a refit work order through a new `UnitMutationOp::InstallComponent` (Q-UM-10 A). |
| D-UM-35 | (*Amended by R-B*) The classes are 0 (1,000 ms, 400 ops, 8 rows, 0 odds rows, 1 slot, 16 keys, 1 profile), I (1,000 ms, 1,500, 12, 2, 2, 32, 2), II (500 ms, 3,000, 16, 4, 4, 64, 3), III (250 ms, 4,500, 24, 8, 6, 128, 4); lore facts in `[tacsim.minds]`, the harness the gate; the same for every role. Chassis declare their cores in content. |
| D-UM-36 | (*Amended by R-B*) Each sim task has a deterministic pool `max(120,000, 500,000 − 1,000 × units)` ops per sim second, admitted by class ceilings. In the arena: split evenly per side, first-come within a side, checked at `SIM MIND`/`SIM UNIT` (the later choice refused with the arithmetic) and at check-out by name. On a region map: admitted at the startup in the hangar or base, never mid-sortie, a hull already out never evicted. The pool is visible in the fiction (Q-UM-2 A). |
| D-UM-37 | (*Amended by R-A and R-B*) In the OOC sim a pod emulates the chassis's content-declared core and the lead may set the AI fill's class (Q-UM-3 A); the OOC sim never reads owned IC hardware; owned hardware matters only on IC sorties. |
| D-UM-38 | (*Amended by R-B and R-C*) Three legal classes enforced by registered host functions: Doctrine (Registry only, free, perpetual; Standard Doctrine at launch, role doctrines with their stations), Discretion (own hull's station channels, licensed by term), Frontier (never mountable, never a principal or an order's author; in the fiction it advises human offices). |
| D-UM-39 | A licence is a term lease (≤90 IC days) in seats counted as concurrent sealed sorties, naming a warrant principal, with Active, Grace (3 IC days), Lapsed, Revoked (Registry only), Recalled; renewal is a fresh auto-drafted E4 contract. (*Amended by R-B*:) a lapse never touches a sealed sortie and drops the hull to doctrine at its next seal, told only to its own principals. |
| D-UM-40 | Licences are transferable unless the author marks the listing otherwise; `InstrumentKind::Entitlement`'s doc comment is amended to say so for catalogue entries that permit it; sales and transfers between one account's characters are refused through the lobby's `AccountId` oracle; a licence sold during a sortie it seals moves at that sortie's end. Charters trade as title transfers. (*Amended 2026-10-04 after review, finding 7*:) a sale while sorties it seals are out is **PendingTransfer** — no new seal admits the licence for either party, sealed sorties close under the seller, settlement is atomic at the last close or at the earlier of the term's end and `transfer_settle_ic_days` (default 3), still-open sorties hold their seats until they close and drop to doctrine at their next seal (§10; the parameters Q-UM-18's). |
| D-UM-41 | A governed core refuses at the seal an unfiled digest, an unlicensed line, an over-class mind and an unlawful slot; the sim task never reads a licence. (*Amended by R-A*:) every OOC sim pod is governed and checks the character's sim set (D-UM-49) in place of a seat; the sim governor admits the character's own unfiled drafts, and every other rule (the lawful slots, the floor, the class against the pod's core) applies there as IC. |
| D-UM-42 | (*Amended by R-B and R-D*) Contraband is ungoverned cores (an unchartered refit leaving provenance; heirlooms; greys) running unfiled, over-class or unlicensed minds and, armed, unlawful slots (Q-UM-6 A), detected only by evidence through five channels (renewal attestation, custody inspection, the black-box seal lines, victim-ledger testimony, the author's quirk claim, D-UM-55) with seizure, fine, revocation and Tycho voiding cover; no contraband cut ships before its detection channel, and none before IC sorties exist. |
| D-UM-43 | (*Amended by R-B*) The card is a mind-declared schema from a closed, role-neutral widget vocabulary, refused not clamped; a section driving a station the hull lacks is shown inert; Standard Doctrine's stock schema maps onto `ReflexConfig` so its Line preset on its defaults is today's machine (withdraw 35, stand 17.5, search 60, weights 100/40/30, min_odds at the floor, no reserve). |
| D-UM-44 | (*Amended by R-B*) Standard Doctrine's six combat presets (Line, Sentry, Sniper, Skirmisher, Hunter, Rearguard) are named cards only; one to four profiles by class, sealed together, switched by `MIND PROFILE` as a recorded intent at most once per 10 s. Pilot-plus-mind channel splits (ASSIST, `MIND ON|OFF <channel>`) are deferred behind T-54 (Q-UM-11 A); v1 modes are NONE and the whole-unit mind acting on every channel its hull's stations grant (holding no lease), and a seated pilot's SOP fire on an armed hull comes from the core's native slots. |
| D-UM-45 | (*R-B*) The seal is per **sortie**: from the startup that takes a unit out of a hangar or base to its power-down in a hangar or base (the same or another), or its destruction or recovery. A power-down in the field does not end it; a pilot's exit and a licence lapse never do, and an engagement or a match on an IC map never ends it; in the OOC arena the match's end, when custody returns to the pod, is the pod's power-down and closes the sortie, journalled `SortieClosed`. The seal is taken at the principal's dispatch, never by the mind, whose first `startup()` executes that dispatch inside the sealed sortie. In the arena the pod is the hangar and `BOARD` then `STARTUP` its exit, so the seal is computed at check-out on `BOARD` (at `Boarding → Running` for the fill) and is the sortie's from `STARTUP`; on a region map it is computed at the dispatch in the hangar or base, ahead of the `STARTUP` there. Nothing in this document assumes a unit is armed, in a match or on the arena map. |
| D-UM-46 | (*R-B*) The sortie is the unit of record: `MindSealRecord` is keyed by `(unit, sortie)`; the `MatchRecord.minds` map carries, per unit, the seal of the sortie the match lies within, and the match ledger's `MindSealed` lines cite the sortie and its `SortieClosed` lines close it at the match's end; IC, each unit's black box opens each sortie with `MindSealed` and closes it with `SortieClosed` (power-down in a base, destruction or recovery), and everything on the way cites the sortie. Memory, the drop to doctrine and `destroy_core` last for the sortie. |
| D-UM-47 | (*R-B*) A mind's outputs are the unit's **station verbs** under the F3 channel model: the mind holds no F3 lease and is never a lease holder; it acts on the arena epoch table's channels of the stations its hull has, its seal names and no seated holder occupies, under the seal's warrant principal; the arena's closed `Channel` and `mush-sim`'s open `ChannelId` meet by wire word (`helm`, `sensors`, `fire`), written down in M1a. A function whose channel the seal lacks is refused `Channel` at landing. The cockpit's combat verbs are the helm, sensors and fire stations' set on a combat hull, one hull's set among others; a future station (mining, cargo, building) adds its `ChannelId`, a new arena `Channel` member (a reviewed hash, snapshot and `Channel::ALL` diff), its human verb, its mind function, its view fields and its kernels' fees in one cut. The fire law and condition slots are the weapons station's; an unarmed hull lacks them and nothing else changes. |
| D-UM-48 | (*R-B*) Stock minds, cards, presets, profiles, proving scenarios and doctrines are **per role**; Standard Doctrine and its six presets are the combat role's, the first that exists. On a hull with no weapons station Standard Doctrine's fire half is absent, not idled — it drives, senses, follows orders, holds, withdraws and returns — and the twin corpus covers such hulls. A mind's header declares its role; role doctrines join the Doctrine class with their stations. |
| D-UM-49 | (*R-A*) In the OOC sim a character has **exactly the minds they hold IC** — Doctrine, their own minds and drafts, the lines they hold live licences for — at no cost, and none they do not hold. The sim reads those holdings and never writes them: no seat is taken, no term runs, nothing is posted; a sealed licence stays sealed (no source, no `mem`); a lapse, or a sale once agreed, removes a line from the seller's set at the next sim seal, and the line joins the buyer's at settlement (2026-10-04 after review, finding 7); a dependent owner's licence (D-UM-57) leaves the set with its parent. The side lead's fill flies only the lead's holdings, and under Q-UM-15's default only Doctrine (as does any released hull) where the other side has a human pilot. No mind, author or listing is named to anyone else in the sim, before or after: during selection only the slot's own character (and, for the fill, its side lead) reads which mind a slot carries, everyone else reads `a mind` or nothing. |
| D-UM-50 | (*R-A*) The OOC sim crosses **no IC boundary**, IC money included: no licence is ever bought for the sim, the author market and every licence, levy and royalty are IC only, and sparring against another author's mind is an IC service not offered in the OOC proving ground. This is this decision's own rule, extending doc 16's framing of the arena as an OOC construct; D-16-3 (the durability rule, as amended at 16b cut AF) stands unchanged and says nothing of IC posting. OOC matches post nothing to the IC economy ledger or any IC record, consume nothing IC and read nothing IC but D-UM-49's holdings; their own match ledger is OOC. D-16-3's row gains this no-IC-crossing clause in M5 (§19). The proposal's `system:tacsim.sim` principal is dropped. |
| D-UM-51 | (*R-A*) The **Ceres Simulation Center** is named as the future IC venue: a mixed-use training and recreation simulator on Ceres running the same arena machinery, where IC payouts may ride on simulated fights that consume no real IC units, and where IC services such as sparring and allotment sales can live. It is distinct from the OOC centre and is not designed here; it belongs to the corridor build-out. |
| D-UM-52 | (*R-C*) Only a **character principal** issues `ORDER`. A character may be human-played or AI-played; an NPC lance-leader is a person under the Warrant and orders as a human does; a unit mind is never a principal, at any class. `Order.issuer` keeps its meaning, the unit whose contact scope a target hint is written in, and is never a character; `Order` gains a separate principal field, `by: CharacterRef` (an IC character, or an OOC sim persona), a record field not folded into `state_hash`. The check sits at the world and bridge admission, where seats map to characters and the deliberation bridge submits a leader's order, not in `apply_intent`: an order whose `by` names no character entitled to order that squad is refused there, never merely one whose source is a mind; `apply_intent` still refuses `ORDER` from `IntentSource::Mind` with `Channel`. |
| D-UM-53 | (*R-C*) W7's deliberation is reframed as an **NPC lance-leader** — an AI-played character — issuing `ORDER` under their own warrant through the existing path, the order carrying that leader in `by` (D-UM-52) while `issuer` stays `None`, the squad's commander unit (`deliberation/request.rs:506`, `arena_bridge.rs:3904`). The leader is never a `Controller` value: it **rides the squad's commander unit** (the lowest unit id, `deliberation/squad.rs`), which stays `Controller::Mind`, flown by its sealed mind; the leader's hints are written against that hull's own contact set, today's scope unchanged, and the leader is down when that hull is wrecked, so the squad drops to doctrine as today. Cut L amends `squad.rs`'s "not a role" doctrine to say the commander is where the leader rides, and mints one leader per AI fill squad, so every fill squad that deliberates today still does; an IC squad dispatched with no character leader, human or AI-played, gets no deliberated orders and runs on doctrine. In the OOC sim each fill leader is a **sim persona**, minted for the match, with no IC character record or warrant, touching nothing IC (D-UM-50). When this lands, doc 16 needs: W7's row reworded to "NPC lance-leaders are people; their orders reach the lance over the relay; deliberation is optional", its §6 Deliberation and Provider-failure paragraphs restated per lance-leader (requested per leader, the order's author the leader, the lapse sentence in the leader's name), the frontier-mind lease recorded as advice to offices and never an order's author, and an as-built §11 entry. Doc 16 is not edited until then. |
| D-UM-54 | (*R-D*) Player minds may fly NPC organisations' hulls only under a licence the author released through a listing (sealed, open or open-resale); procurement buys such a licence like any holder (budget, trial rating per credit, standing filter, 40 percent diversity cap; seats are its concurrent sealed sorties). A player mind on an organisation's hull without one is contraband. |
| D-UM-55 | (*R-D*) **Quirk recognition** is an earned-evidence channel for the contraband loop: an author who recognises their own mind's behaviour in an organisation's hulls, from what their own sensors earned, may file a claim with the Registry; the engine never confirms or denies a likeness, and the claim is testimony that opens a custody inspection or a black-box read of the suspects' seal lines, which alone establish the fact. Published as `minds.claimed`. |
| D-UM-56 | (*Proposed 2026-10-04 after review, finding 5; pending the owner*) Filing and listing are separate acts: `@mind/file` files a revision with the same conformance and fee and a public Roll entry offering no forms; filing is not R-D's release (procurement buys only a listed line); filing issues the charter owner a priced-at-nothing, non-transferable **owner's licence** on the audit term renewed by attestation, following the charter's title; an unfiled draft runs IC only on an ungoverned core (contraband). |
| D-UM-57 | (*Proposed 2026-10-04 after review, finding 5; pending the owner*) An `open` fork is a derivative draft filed privately only while the holder's parent licence is Active or Grace, under a lineage check (parent live and held, chain depth ≤3, royalty cap); its owner's licence is **dependent** on the parent's and lapses with it, the fork becoming source-sealed (last filed revision runs, no new revision) until the parent line is held again — the default of Q-UM-17; `open-resale` additionally allows listing; charters are refused between one account's characters. |

---

## 21. Owner answers (2026-10-02)

1. **Q-UM-1, names.** Accepted: Standard Doctrine, the drive ROM, the doctrine, line and command
   cores, Discretion, the Roll of Minds, Basin Charter Instruments.
2. **Q-UM-2, the pool in the fiction.** A: visible; `SIM MIND` prints the arithmetic and IC an
   allotment is a resource an operator sells or rations (D-UM-36).
3. **Q-UM-3, sim hardware.** A: a pod emulates its chassis's declared core and the lead may set the
   fill's class (D-UM-37).
4. **Q-UM-4, IC money for OOC sim use.** Neither A nor B (R-A): the OOC sim crosses no IC boundary;
   a character flies exactly the minds they hold IC, at no cost, and none they do not; the market is
   IC only; the Ceres Simulation Center is the future IC venue (D-UM-49, D-UM-50, D-UM-51).
5. **Q-UM-5, may a mind ever ORDER?** A with a differentiation (R-C): never; only a character
   principal, human- or AI-played, orders, and W7's deliberation becomes an NPC lance-leader
   (D-UM-11, D-UM-52, D-UM-53).
6. **Q-UM-6, contraband autonomy.** A: an ungoverned core may arm unlawful slots as the setting's
   central, rare, provable crime (D-UM-42).
7. **Q-UM-7, SSH push and pull.** A: design 02 §5.6 is amended with a narrow `mind push|pull|check`
   exec allow-list, public-key auth only (§15).
8. **Q-UM-8, player minds on NPC armies.** A with a condition and a hook (R-D): only under a licence
   the author released through a listing; an author's recognition of their mind's quirks is an
   evidence channel (D-UM-54, D-UM-55).
9. **Q-UM-9, floats in scripts.** A: allowed in expressions, INT at every boundary, behind the
   cross-platform hash CI test (D-UM-28).
10. **Q-UM-10, the install op.** A: `UnitMutationOp::InstallComponent` in `mush-tactics` under the
    M32 plan/apply/journal discipline (D-UM-34).
11. **Q-UM-11, the T-54 cut's place.** A: per-channel leases and ASSIST land after M8 as their own
    cut (D-UM-44).

And one standing ruling on the seal that no question asked (R-B): the seal is per sortie, from the
startup leaving a hangar or base to the power-down in one, and nothing in this document assumes an
armed unit, a match or the arena map (D-UM-45 to D-UM-48).

**Questions the rulings raise (defaults marked; the default holds until the owner says otherwise).**

12. **Q-UM-12, the ROM on an unarmed hull before its role doctrine exists.** **Default:** the ROM runs
    Standard Doctrine with its fire half absent (it drives, senses, follows orders, holds, withdraws
    and returns), which is what today's reflex does with a hull that cannot fight. **Alternative:**
    an unarmed hull may not be sealed with a mind or sent out unseated until its role doctrine exists.
13. **Q-UM-13, whose holdings in the OOC sim: the character's or the account's.** The ruling says
    "a player" has the minds they have IC. **Default:** the character who boards the pod, so the sim
    set is exactly that character's IC library and matches the alt-laundering refusal of D-UM-40.
    **Alternative:** the union over every character of the account, since one human already knows
    what all their characters hold.
14. **Q-UM-14, may an author keep a listing away from NPC procurement?** **Default:** no separate
    flag at launch; publishing a listing is the release R-D names, and an author who does not want
    organisations flying a mind does not list it. **Alternative:** a listing term `orgs=off` that
    procurement's `AvailabilityProvider` honours.
15. **Q-UM-15, may a licensed mind fly against humans who do not hold it in the OOC sim?** R-A gives
    a character access to exactly the minds they hold IC. A side lead's fill flying a licensed mind,
    or a pilot's sealed mind flying their released hull, against opposing humans who do not hold that
    mind lets those humans learn its habits OOC. That is the sparring D-UM-50 refuses in the proving
    ground, a behavioural OOC-to-IC leak, and it would seed D-UM-55's quirk "recognition" from OOC
    play. Not naming the mind does not stop the behaviour being learned. **Default:** in a match where
    the other side has a human pilot, nothing but Doctrine flies unseated on either side. The fill
    flies Doctrine at the lead's chosen preset. A pilot's own sealed mind stays dormant on release,
    and the hull goes to Doctrine on its fallback preset. The pilot is told once at `SIM START`
    (`Humans on the other side: your fill and any released hull fly Standard Doctrine.`). A
    narrower rule ("only minds every opposing human holds") is not the default, because its refusal
    would tell the lead something of the opponents' IC holdings. A match against fill alone (M5's
    gate transcript) and the proving ground are unaffected. **Alternative:** facing a mind is not
    access under R-A, since flying, tuning and reading it are, and the residual is accepted as
    §23's risk entry describes.
16. **Q-UM-16, the condition slot against 16c Q45.** The owner ruled on Q45: no armed trigger and
    nothing that fires for the pilot, with parity coming from the machines paying the seat's pulse.
    **Default:** a condition slot is the principal's written shot decision (target class, odds floor,
    aspect, reserve, announced at every fire and hold), not Q45's fire-when-ready trigger. It stands
    as D-UM-16 and D-UM-17 write it, for pilots and minds alike. Parity runs both ways (§5), and doc 17
    closes Q45's row. **Alternative:** slots arm for minds only; a seated pilot's every shot is a typed
        `FIRE` and pays the seat's pulse, and the cockpit's `FIRE <weapon> WHEN …` and M1b's slot
    transcript are dropped.
17. **Q-UM-17, the fate of an `open` fork whose parent licence ends** (raised 2026-10-04 after review, finding 5;
    D-UM-56, D-UM-57). **Default:** the fork becomes source-sealed — its last filed revision still runs
    and seals, no new revision may be filed — until the holder holds the parent line again; the
    author's self-use runs through a free owner's licence that filing issues, and filing never lists.
    **Alternatives:** the fork is deleted at the parent's lapse; or it persists unlicensed (which makes
    `open` a one-time sale of the source and lets a fork outlive a sold parent).
18. **Q-UM-18, the PendingTransfer parameters** (raised 2026-10-04 after review, finding 7; D-UM-40). **Default:**
    settlement at the last admitted sortie's close or at the earlier of the term's end and 3 IC days (the
    Grace length), and sorties still out keep their seats until they close so open sealed sorties never
    exceed seats. **Alternatives:** a longer or no deadline; seats freed at settlement (which lets one
    licence fly the seller's and the buyer's hulls at once).

---

## 22. Build order (milestones and gates)

Each milestone keeps the proposal's content and gate; a line in italics under each says what the
sortie generalisation (R-B), the OOC boundary (R-A), the character principal (R-C) and the
procurement condition (R-D) change in it. The arena remains the first and, until the corridor's
region maps exist, the only place each milestone is proven.

**M1a — the seam, no Rhai.** Extract `arena/motor.rs` with its tests; move the row formatter into
`arena/view.rs`; add aspect, odds, `reads_dead`, order and `t_ms` to `Frame` and the keyframe
(`FRAME_VERSION` 2, bot parser and goldens updated); `Controller::Mind`, `MindState` with the notice
buffer, `DueKind::MindThink` and `MindLand`, `apply_machine_intents` with the Emcon arm,
`IntentSource::Mind`, `GOTO` as a cockpit verb, the hint re-keying; the native T1 stock mind; the
motor audit tests. Gate: every reflex, step and cross-tempo fixture green; the cross-tempo fixture
with two T1 units byte-identical at 5,000, 1,000 and 500 ms; both harness lanes hold their lenses;
T0 vs T1 over 30 pooled matches inside a Wilson interval containing 50 percent; the six divergences
recorded in doc 16 §11. Landed 2026-10-03 at `35af96dc`; see 17b section 3, row M1a.
*Sortie and stations: odds and `reads_dead` are built only for a hull with a weapons station and the
keyframe drops the weapons block on one without; the mind's outputs are grouped by channel and an
ungranted channel refuses `Channel`; T1 runs an unarmed hull with its fire half absent and a fixture
proves it; `IntentSource::Mind` is refused `ORDER`.*

**M1b — slots and sliders, no Rhai.** `FireCondition` slots with native evaluation at `WeaponReady`
(its no-op arm filled), sensor refresh and landing; `hold_fire` in `resolve_fire`; `Unwarranted`; the odds floor, with `Unlawful` at every arming and cited fire against the configured floor (review 2026-10-04 after review, finding 2); as its first sub-cut the same-millisecond ordering fix (a landing's zero-flight impact applied before that millisecond's thinks, finding 3) and `MindView.scan_tables` (finding 4); `FIRE <weapon> WHEN …`, `WEAPONSTATUS` arming, attribution and held-shot sentences;
the stock card lifted from `ReflexConfig` as per-unit terms with the six presets, refuse-not-clamp,
and an owner-facing `@mind/set` on T1. Gate: a fixture shows a conditioned shot resolving on the `WeaponReady` millisecond, not a pulse later; `FIRE cannon WHEN odds >= 5` refused `Unlawful` as typed; a beam landed at a think's millisecond seen by that think at three cadences; a line-mode transcript of `FIRE cannon WHEN odds >= 60
AND aspect = side,rear` firing only on flanks while a typed `FIRE` still fires; a 30-match pooled
measure of `mush-bot` against a conditioned T1 reported with Wilson intervals; the harness lenses
unchanged. Landed 2026-10-05 at `d2d49389`; see 17b section 3, row M1b.
*Sortie and stations: slots and the rule language are the weapons station's; a fixture seals the
stock card on an unarmed hull with its fire-discipline section inert and no slot, and the seal does
not refuse; 16c's `Q45` row is marked closed by D-UM-16 and D-UM-17 in this cut (§5, §19).*

**M2 — the Rhai runtime.** `crates/mush-mind` (engine builder, op-and-fee budget, canonical memory,
seal cache), `crates/tacsim/src/minds` (view types, kernels, the station functions, `RhaiRunner`),
`content/minds/standard.rhai` with its manifest (Line = `ReflexConfig` defaults), `DrawLabel::Mind`.
Gate: sandbox tests (identical budget trip on two runs; no time, eval or keyword-function symbol
resolves; memory canonicalisation; type faults; the cross-platform hash test; a maximal valid Class III memory — 128 keys of 32-int arrays, and of 32-byte ASCII strings — round-trips with no data-size fault while an over-count is a `memory_check` fault and a runaway `s += s` trips the engine limit, finding 6 of 2026-10-04 after review); Gate A, T2 ≡ T1 on the
30-run corpus, the cross-tempo fixture and whole-match hashes. Landed 2026-10-06 at `96887076`; see 17b section 3, row M2.
*Sortie and stations: the manifest declares the role (`combat`); the twin corpus carries unarmed
hulls and Gate A covers them; a call to an ungranted station function is a refusal in the trace, not
a fault.*

**M3 — hardware, seal, record.** The `core` content kind and `ComponentRole::Core` with
`CoreProfile`, `degrade_core` and `destroy_core`, the `MindSeal` on `CustodyUnit`, the pool with
admission at `SIM MIND`/`SIM UNIT` and check-out, the `MindSealed`/`MindFault`/`MindDropped` ledger
kinds, `MatchRecord.minds`, the source store, snapshot rows and resume (`MindState.pending` carrying the think's answer, D-UM-23 as amended 2026-10-04 after review; the seal capturing the odds floor, finding 2), the dormant row across possess and release. Gate: the label-set and verb-set contract diffs reviewed; a resume between N
and N+1 reproduces the hash sequence; an over-subscribed roster refuses by name; three injected
faults drop a unit to the ROM; `destroy_core` drops one at its next landing.
*Sortie and stations: the seal at `BOARD` becomes the seal at the dispatch ahead of the `STARTUP`
leaving a hangar or base, the sim's `BOARD`-then-`STARTUP` its first implementation; the seal is keyed
`(unit, sortie)` with a sortie id minted at the seal, and `MatchRecord.minds` carries the sortie's
seal; the drop and `destroy_core` last for the sortie (in the sim, the match, whose end closes it); a
re-seat after a release is asserted not to re-seal. The arena's `SortieClosed` is built here: the
gate asserts one `SortieClosed` line per unit when custody returns to the pod at the match's end (or
at the unit's destruction), and that a resume across a changed `MIND_ABI` is refused by name.
Region-map closes, startup admission and the IC resume rule wait for the region sim task.*

**M4 — the P3 gate and the flip.** `tacsim_harness_arena_minds` (stock CI, stock authored, ceiling
authored) with jittered wakes and the arena-task pool, which awaits any outstanding think batch before a snapshot is encoded (D-UM-23 as amended 2026-10-04 after review); AI fill flipped to Standard Doctrine on Rhai;
`Controller::Reflex`, `ReflexTick` and the decision half of `reflex.rs` deleted; `NotReflex` →
`NotMachine`. Gate: every lens holds three times running on the blessing host through
`scripts/harness-weekly.sh`; the deterministic block committed to the baseline; if the stock lane
fails, the fast-path decision (T1 in production for Standard) recorded in doc 16 §11 before the flip.
*Sortie and stations: the cohorts' non-fighting units fly as unarmed minds, so the flip covers every
hull the harness has, not only its fighters; nothing else changes.*

**M5 — the pilot's experience.** `SIM MIND`, `SIM AI MIND`, sealing at the pod's hangar exit, the
`MIND` cockpit verb and block on line mode, TUI and web at 120 columns, profiles and `MIND PROFILE`,
the rule language compiling to slots with English written back, release-past-grace to the sealed
mind, the `Your mind` block of `SIM REPORT`, the two hints, `help minds`. Gate: a line-mode transcript
on the local rig of a pilot sealing a mind, exiting, and the tank flown by it; goldens and the bot
parser updated; fog asserted (during selection only the slot's own character, and the lead for the
fill, reads a slot's mind; everyone else reads `a mind` or nothing; no mind named after the start or
in the record).
*Sortie and stations, and the OOC boundary: `SIM MIND` and `SIM AI MIND` list and accept only the
character's (or the lead's) IC holdings and Doctrine (D-UM-49), with a fixture that a mind the
character does not hold is refused and that the sim seal takes no seat and posts nothing; the
cockpit's sentences speak of the sortie where a match is not meant; the `MIND` block takes the
weapons block's place on an unarmed hull; the sim governor admits the character's own drafts; with a
human on the other side the fill and released hulls fly Doctrine (Q-UM-15's default, unless the
owner rules otherwise); D-16-3's row in doc 16 gains D-UM-50's no-IC-crossing clause (§19).*

**M6 — the workshop, the proving ground and the bench.** The `@mind` switches, content-addressed
revisions, `@mind/check` on canned pictures, schema-rendered cards, the web workshop, the TUI card
screen, SSH push and pull (Q-UM-7 A); the trace ring and `arena.mind`; the proving executor on its
pool with quotas, both-sides seeds, `wilson_95` in the library, the report; the seat-intent tape
under its bound; the bench with closed-loop and open-loop sessions, STEP, BACK, TRACE, WATCH, EVAL,
DIFF, SWAP, POSSESS, ACT AS; `help mindcraft`, `help proving`. Gate: the `mush-bot` pilot (Picture,
Pilot, Doctrine) ported as a Rhai mind seals, flies and checks within Class II; against Standard over
30 pooled muster matches it lands inside the bot's own pooled interval; a bench re-run reproduces a
proving match's hash at every step; re-stepping every think of a kept OOC match reproduces its
intents and hashes; two running proving arenas leave the live p99 unchanged.
*Sortie and stations, and the OOC boundary: proving opponents and `DIFF`/`SWAP` minds are Doctrine,
the character's holdings and `mirror`, never another author's listing (no sparring in the OOC
proving ground); `/check`'s canned pictures run on an armed and an unarmed hull; the scenario set is
content so a role's scenarios can join later.*

**M7 — the Roll and the market.** `crates/mush-roll`: drafts, filing conformance and the rated
bench, the digest, charters through `issue_title`, lines with follow and pin, licences as seat-term
leases settled by E4 contracts with the levy, venue and owner split, auto-renewal, Grace, lapse to
doctrine at the seal; Entitlement listings through tacsim-registered tables; sealed, open and
open-resale terms; transferable licences with the alt refusal; the house vendor catalogue; IC cores
as titled assets with dies, lineage, the importer, `core_integrate@1`, the install op and the
Class III end-user entitlement. Gate (amended): on the local rig one character publishes a sealed
mind; a second character of another account buys it IC, and the line appears in that character's
OOC sim set at no cost with no seat taken and nothing posted, and flies there sealed (card and
traces, never source); the buyer resells the licence with a balanced ledger and the line leaves the
buyer's sim set at the next sim seal; an alt-to-alt sale is refused; a lapse falls to doctrine at the
next seal with the line shown only to its principals; a derivative's settlement pays its parent chain exactly; a filed-but-unlisted mind flies its author's own hull and is never bought by procurement; a privately filed `open` fork runs and stops accepting revisions when its parent licence lapses; a two-seat licence resold with one sortie out settles once, as D-UM-40's PendingTransfer says (review 2026-10-04 after review, findings 5 and 7, the D-UM-56/57 and Q-UM-18 defaults pending the owner).
*Sortie and stations, and the OOC boundary: the purchase is IC only and never "for the sim"; seats
count concurrent sealed sorties; the lapse rule is tested against a sealed sortie that outlives the
term; flying a bought mind on an IC hull's sortie is added to the gate when IC sorties exist.*

**M8 — contraband and procurement (gated on IC sorties).** The governed bit (an ungoverned core alone skips the arming-time floor check of D-UM-17 as amended 2026-10-04 after review, its unlawful arming journalled), ungoverning refits,
heirloom and grey provenance, renewal attestation, custody inspection verbs, black-box seal lines,
seizure, fine, revocation, Tycho's cover condition, the `minds.unwarranted` canon event and the Red
Dies front; the quirk claim and `minds.claimed`; NPC procurement with the diversity cap. Gate:
observer-rule leak tests prove an ungoverned core is learned only through the five evidence channels;
the zero-PC test in which an NPC org re-equips from the Roll on its own budget with no charter over
the cap.
*Sortie and stations, and R-D: gated on IC sorties rather than IC combat alone, since an unlicensed
mind on an organisation's miners is contraband too; procurement buys only listed lines (a fixture
that an unlisted mind is never procured); a quirk-claim test proves the engine confirms nothing until
an inspection reads a seal line.*

**L — the lance-leader (independent of the M-series; after M1a).** An NPC lance-leader is an
AI-played character attached to a squad, riding its commander unit (the lowest id), which stays
`Controller::Mind`. `Order` gains `by: CharacterRef`, and `issuer` keeps its hint-scope meaning
(`None`, the commander unit, for a leader's order). Deliberation is requested per leader, and each
deliberated `ORDER` carries the leader in `by` through the existing path. The world and bridge
admission refuses an order whose `by` names no character entitled to order the squad; a typed
`ORDER` takes the seat's character as `by`. One sim-persona leader is minted per AI fill squad, so
today's fill squads keep deliberating; an IC squad with no character leader runs on doctrine.
`squad.rs`'s "not a role" doctrine is amended to name the commander as the leader's hull, and the
"mind link dropped" sentence becomes the leader's orders lapsing. Gate: the deliberation fixtures
pass with `by` present and `issuer` unchanged, and the hint resolves in the same unit's contact set
as before; a forged order with no `by`, and one from `IntentSource::Mind`, are refused; a wrecked
leader's hull drops its squad to doctrine as today; the harness with the LLM disabled is unchanged
and its deterministic block does not move (`by` is not hashed). Doc 16's W7 row, §6 paragraphs and
§11 are amended in this cut as D-UM-53 says. Landed 2026-10-04 at `3ef6e34a`; see 17b section 3,
row L.

**After M8.** The T-54 cut: per-channel K6 leases, a second seat on the hull, ASSIST and `MIND
ON|OFF <channel>` with a second hashed rate counter, mercenary `mind.warrant` delegation
(`Delegation.expires_at`). The first non-combat station (mining or cargo) with its channel, verb,
view fields, kernels, role doctrine, presets and proving scenarios, under D-UM-47 and D-UM-48. The
region sim task's region-map `SortieClosed`, startup admission, long-sortie snapshots and the IC
resume rule across an ABI change (§13). Real multi-second
weapons, after which the slot and cadence measures of M1b are re-run. The Ceres Simulation Center
(D-UM-51) with the corridor build-out.

---

## 23. Risks

- **The view's own cost is unmeasured.** `mind_view` for 75 units per substep with `aim_at` plus
  `resolve_seen` per odds row is a fire trace each and could cost more than the think. The lane
  reports `view_us` and `odds_rows` per class is the lever.
- **One host's interpreter speed.** 20–24 ops/µs is ALIEN18; dev1 may be markedly slower, so fees
  and the pool calibrated here could under-budget production. The lane prints measured ops/µs; the
  pool and `cpu_lens_ms` are knobs; the op budget itself is deterministic everywhere.
- **The authored cohort's window headroom.** 14–18 ms of 20 is used; thinks stay out of the window
  only at single-substep wakes, and jittered or Quiet wakes run them inline — which is why the lane
  forces them and why the CPU lens is separate.
- **A saturated shared pool** shows as late wakes, which push pulses off the lattice (results are
  unchanged, wall time is not). Backpressure is bounded by the pool ceiling, and `pool_wait_us` is
  watched.
- **Op counts are the evaluator's.** A rhai patch can change counts and which thinks trip, breaking
  re-simulation of older matches; hence the exact pin, the ABI string, and, in the arena, refusal
  across versions. An IC sortie out across an upgrade is kept and its minds drop to the ROM (§13).
- **Floats across platforms** are deterministic in theory for basic IEEE operations on one binary
  but unproven between the Windows blessing host and the Linux build; the CI test is mandatory, and
  Q-UM-9's B remains the fallback.
- **Observer leakage through kernels** — odds expose the silhouette (gated on classification);
  `line_to`, the nav pickers and the condition evaluator leak if a later change traces to live poses
  rather than plots. Each kernel has a hold-the-plot-move-the-truth test, and each future station's
  kernels inherit the rule.
- **Migration blast radius.** `reflex.rs` is 7,042 lines with about 1,800 of tests; `step.rs` and
  the harness counters assume `ReflexTick`; Gate B may show today's tuned machine stronger than its
  observer-pure port and force a retune before the flip.
- **Conditions will out-shoot typed `FIRE`** under the 1 s placeholder weapons, and whole-unit minds
  plus slots may make "arm slots or lose" the meta before real weapons arrive. Intended by P1; the
  measures are re-run when the placeholder goes.
- **The IC-only market's cold start.** With no sim market (R-A), an author's only buyers are IC
  holders, and IC sorties do not exist yet; a mind-smith may have nothing to sell into for a long
  time. The levers: M7 is ordered with the corridor rather than ahead of it; Doctrine stays free; the
  proving ground lets authors work and measure before they can sell; NPC procurement is demand with
  no player on the other side; the Ceres Simulation Center is the first IC venue where a mind's worth
  shows without risking a real unit.
- **Station-verb breadth.** Every new station — mining, cargo, building — carries a channel, a human
  verb, a mind function, view fields, kernels with fees, a role doctrine, presets, proving scenarios
  and twin-fixture cases, and the closed-verb rule makes each a reviewed cut. The surface grows with
  every role; one station per cut, verbs for humans first, is the discipline.
- **Long sorties.** An IC sortie may last hours: a seal taken under a licence that lapses an hour in
  holds for the rest of the trip, memory grows to its cap and stays there, and a fault-dropped hull
  stays on the ROM until it reaches a base. The term cap, the grace and the seat count bound the
  first; the per-class key cap the second; the third is intended, and the region sim task's
  snapshots are sized for it.
- **Residual behavioural leakage through the OOC sim.** A side lead's fill or a released hull flies
  minds its character holds IC, so an opponent could learn a mind's habits OOC and later "recognise"
  them IC. Not naming a mind does not stop that. Q-UM-15's default closes the leak against humans:
  where the other side has a human, nothing but Doctrine flies unseated. The sim never names a mind,
  author or listing before or after; recognition IC proves nothing without an inspection (D-UM-55); and
  Q-UM-13's default keeps the set to one character. If the owner takes Q-UM-15's alternative, this
  residual is accepted as it stands.
- **Quirk claims as harassment.** Claims cost the Registry an inspection and can be used to probe a
  rival's hulls. A claim is evidence for an inspection, never a verdict, and what an inspection
  reveals to the claimant is only whether their own revision ran.
- **Market balance and monoculture.** One dominant mind sold to everyone, or flying most NPC hulls,
  flattens play; the levers are classes, the pool, the diversity cap and content tuning, which may
  not suffice.
- **Alt and listing abuse.** Laundering through sales (the same-account refusal), sealed-source
  leakage through an author's own `trace()` (their choice), fraudulent claims on a listing (only
  system-run proving figures appear on one).
- **Money machinery ahead of demand.** Licences, loadouts and seals are authoritative action-legality
  families; the Roll, levies and procurement arrive before IC sorties exist. M7 and M8 are ordered
  last and M8 is gated on IC sorties for that reason.
- **Storage.** Source revisions, seat tapes, sortie seals and proving records need retention sweeps
  tied to the twenty-match window, the reference rules of §15 and `tape_max_bytes`.
- **Complexity for new players.** Five rungs, roles, profiles, classes and licences could overwhelm;
  the defaults (no mind on a new slot, Standard on Line for the fill, a three-line first mind) and the
  three-line opening of `help minds` must stay true.

---

## 24. Amendments (as built)

Each cut that builds part of this document records here what it built differently and why, in the
form of doc 16 §11, and a region map's sim task records here any divergence from §2's arena facts.

- **§4 "The motor" and D-UM-3 (the motor extracted, the reflex kept whole)**: as built (17b cut M1a.1, 2026-10-03, `10a5a0ec`): `arena/motor.rs` holds `MotorRow { goal, route, searched_ms, searched_from, avoid, refused, waypoint, from_waypoint, hold_retry_ms }` with `MotorRow::age_avoid` (the avoid TTL sweep lifted from the head of `decide` and called where it stood) and `MotorTable` (the rows, the path budget window and spent, the A\* scratch) as `Arena.motors` for every controller; `follow(&self, id, t_ms, motors, row, here, policy: Fallback::{Waypoint, Notice}, counters, plan) -> MotorOutcome { arrived, unreachable }` reads `arena_cfg.reflex`, and `decide` performs the B-41/Z.2/E.4 give-ups from the outcome in their old order; `step_is_drivable`, `first_step_is_open`, `line_is_open`, `walk_is_open`, `entry_cell`, `far_entry_cell`, `stand_off_cell`, `patrol_step`, `hold_arrived` and `fall_back_to_waypoint` are `pub` as the audited surface (the five audits: hold the plot and move the truth; the pickers read the ground and the own pose; the walks read terrain and the avoid list and never the index; a stop is learned at the `blocked` event; a pilot row ages its stops). Departures from D-UM-3 as written: `DueKind::ReflexTick` is **not** renamed and stripped of `decide` — it stays as T0's cadence until M4, and `DueKind::MotorTick` (M1a.4) is a new kind beside it for a hull no reflex tick drives; `ReflexCounters` is not renamed `MotorCounters` until M4, so the harness baseline keys do not move; `ReflexUnit` keeps the whole decision half (`state, target, no_line, stand_until_ms, quiet_since_ms, rallied, order, last_plot, unheld_since_ms, hunt`), and `reflex.rs` loses only the driving fields (7,042 → 3,981 lines; 4,138 after M1a.6). `set_controller(Pilot)` releases the machine row and the motor row through `Arena::release_reflex` (§2's table says `reflex.release`, which was `e30b9491`'s code), and the controller guard on the motor's readers (`hold_routeless`, `note_blocked`, `refresh_held_avoid`) is replaced by the row: every transition away from a controller releases its row (M1a.5 made that hold for `Mind` too). The hash fold reads the table in the same byte order, two constants read at HEAD before the move pin it (the 1,000 ms replay's first and last hash, the shipped duel's final hash, re-asserted by M1a.4, M1a.5 and M1a.6), and a per-row section folds motor rows with no reflex row; `SNAPSHOT_FORMAT` 2 → 3 with a `motors` section. §2's and §4's `reflex.rs:` anchors (`:845`, `:1247`, `:1464`, `:2862`, `:3118`, `:3616`, `:4567-4640`, `:5911`) point at moved code and are left as they were at `e30b9491`.
- **§3 "One formatter" and D-UM-4 (the row formatter moved)**: as built (17b cut M1a.2, 2026-10-03, `b446b918`): `arena/view.rs` holds `frame_contacts`, `frame_contacts_held`, `range_m_rounded`, `held_range_m`, `settle_ranges`, `own_scope`, `class_word`, `relative_mdeg`, `contact_band`, `ContactRow` with `render` and `glyph`, `frame_lock`, `lock_designator`, `fired_by_default` and the four glyph constants, moved verbatim with their tests — a wider list than §3's four; `cockpit.rs` re-exports them in one `pub use`, and `contact_sentence` (the brief's sentence), `CONTACT_RANGE_STEP_M` and `lock_range_step_m` stay where §3 says. Byte identity is pinned two ways (the fixture scene's `CONTACTS` rows against literals read at HEAD, `contact_sentence` against `view_brief.txt`), by the bot's contract test and by `git diff --exit-code` on the fixtures, goldens and help; no sentence moved. §3's `cockpit.rs:824, :1041, :1062, :963` point at other code now.
- **§3 "Three fields" and D-UM-5 (the frame grows)**: as built (17b cut M1a.3, 2026-10-03, `27e3137a`): `Aspect { Front, Left, Right, Rear }` off `facet_band_seen` on the plot for every hull (the keyframe carries the facet word, `CONTACTS` prints `front`/`side`/`rear`); `reads_dead: Option<bool>`, absent on a hull with no weapons station, from the own kill or a held reading showing destroyed a section the target chassis's **definition** marks fatal (`fatal_section_mask` over the definition, never the live wreck flag, never `contact_reads_dead` as a call, and value-identical to it wherever a reading is held, so §17's fourth divergence was expected to count nought and did), with the fatal section words published beside a held reading on every hull, armed or not; odds per weapon for the two nearest classified contacts (`STOCK_ODDS_ROWS = 2`, class I, the constant M3's core `odds_rows` replaces), the `fired_by_default` weapon's entry first, through the new `Arena::aim_at` (`lock_aim` over any held contact) and `resolve_seen` off the plot, never for an unclassified contact; `FrameOrder { squad, objective: MindObjective, hold_fire, max_range_band, expires_ms }` re-keyed by `rekey_order` to the hull's own **held** `ContactId` or none — D-UM-15 **as decided**; the plan's departure (c), the pure `draw_contact_id`, was withdrawn after review because an id minted for a target the hull does not hold would link a later `X<n>` detection to the ordered target — and `null` on every piloted frame; `t_ms` the state's clock; `frame_for` factored out of `frames()`; `FRAME_VERSION` 2; the keyframe omits `weapons`, `odds` and `reads_dead` on an unarmed hull and gains `bands_cm` per weapon and the order's payloads; `Frame::same_as` compares odds at the whole percent and never `t_ms`; the `CONTACTS` row ends `  side  cannon 62%`, `  side  -` or, unarmed, `  side`; the TUI's compact ` side 62%` tail at the 46-column pane (the extreme row wraps as scan lines do); the web decoder and rows; the bot's `contact_row` and `arena_contract.rs` moving as a declared reader change. `frames_per_pulse_mean` was re-blessed once, 18.867 → 18.875. Open for the lead (17b row M1a): Q2, the odds rows taken by wingmen nearer than the locked hostile; `rekey_order` keeping a held but unclassified row; `MindObjective` carrying no target on `Hold`, `Advance`, `Withdraw`, `Screen` and `Support`.
- **§4 "the pilot's `GOTO`", D-UM-10 and §12 (`DRIVE`)**: as built (17b cut M1a.4, 2026-10-03, `7e45b9b1`): the verb is **`DRIVE`**, not `GOTO`: `mush-core` registers the baseline `goto` with aliases `go` and `move` and `Registry::insert_entry` refuses the collision, so a `goto` selector fails the module at boot (the plan's Q1 at its default; M2's Rhai `goto(..)` function name is unchanged). `IntentKind::Goal(MotorGoal::{Cell, Hold, Off})` is continuous on helm (`as_str` `drive`); a `Seat` goal is counted on the seat window (`[tacsim.arena.intents].max_pending_per_unit`'s meaning widened by doc, D-UM-14's exemption of `IntentSource::Mind` kept) and clears `searched_ms` while a `Mind` or `Machine` goal keeps it; a seat's `HEADING` or `SPEED` disengages a non-reflex motor goal and the motor's own `Machine` orders never, as §4 says; `DRIVE HOLD` is a stop with no goal and no notice; `fan_goal` at the landing is 16b cut AI.1's hunt fan as a motor rule for minds and pilots (T0's own fan untouched); `DueKind::MotorTick` runs `motor_step` every `decide_ms` on the stagger grid while a goal stands, and once at the landing millisecond (§4's "one `follow` at the landing millisecond" delivered); `Fallback::Notice` clears the goal on arrival and on no route and stages `drive_arrived` (`DRIVE: arrived.`) and `drive_no_route` (`DRIVE: no route from here; holding.`), the stock mind performing T0's give-ups and the waypoint fallback one think later off the second; the pre-pulse buffer keeps whichever of a `Goal` and a `HEADING`/`SPEED` the pilot typed last within one pulse, settled by variant. D-UM-10's count is corrected: `COCKPIT_VERBS` was 15 at HEAD (16b cut AP.3's `WATCH`), `DRIVE` makes 16 and `MIND` will make 17; the prefix `dr` is `Ambiguous(drive, drop)` game-wide (pinned and declared in the stanza; an exact-only registration is the lead's call). `SNAPSHOT_FORMAT` 3 → 4. A goal written onto a `Controller::Reflex` hull is overwritten at its next tick except under `Withdraw`, which keeps it as its route home (unreachable from any seat today; the lead's call).
- **§3 "Notices", §4 "A mind never orders", §6 and D-UM-2, D-UM-4, D-UM-8, D-UM-9, D-UM-11, D-UM-14, D-UM-20, D-UM-21, D-UM-47 (the mind seam, no Rhai)**: as built (17b cut M1a.5, 2026-10-03, `6164bc07`), on the programme lead's note of 2026-10-03 that outranks this document's prose where they differ — no Rhai, no `mush-mind` crate, a production match on `Controller::Reflex` and its `state_hash` unmoved: `Controller::Mind` is hash code 2 (D-UM-2's "every unit without a seated pilot" waits for M4's flip; no production path sets it, and a `Mind` unit exists in fixtures and the twin lane alone); `DueKind::MindThink` and `MindLand` run **last** among the due events of their millisecond, landings before thinks, in unit-id order, independent of `seq` (D-UM-20 and D-UM-21 delivered; the one-pass drain superseded by the M1b.0 entry below); the think at `grid_at(unit, STOCK_THINK_MS = 1,000)`, the landing at `t_ms + substep_len_ms()` (250 ms on shipped content); `Arena::mind_view(&mut self, unit) -> Option<MindView>` — not D-UM-4's `mind_view(unit, &mut MindState) -> MindView` — built from M1a.3's `frame_for` through `view::frame_contacts_held` against the mind's own range hysteresis, the contacts truncated to `STOCK_CONTACT_ROWS = 12` nearest-first and the whole view projected at the keyframe's resolution (`bearing_deg`, `relative_deg`, `range_m`, `sight_m`; nothing finer), so T1's `Screen` arc and `Ranges` round where T0 does not; `MindState { think_seq, pending, memory: Vec<u8>, notices (16), ranges, order: Option<FrameOrder>, strikes }` hashed per row with no count prefix and carried by the snapshot (`SNAPSHOT_FORMAT` 4 → 5). **Departure from D-UM-8 and D-UM-20**: the notice buffer is filled by `stage_notice` at staging time, not by a post-hash journal scan, so with mind kinds running last a think can read a notice staged earlier in its own millisecond — one substep earlier than §6 says; deterministic and tempo-independent. D-UM-9 delivered as: more than `MIND_INTENTS_MAX = 16` intents lands nothing, keeps memory, puts the drained notices back and restores the pre-think ranges, and schedules no `MindLand`. `apply_machine_intents` is `Arena::order`'s body factored out, answering `Landed::{Applied, Refused}` per intent with every resolver's `Err` folded in and the Emcon arm calling `resolve_emcon` (§2's "its Emcon arm is a no-op" ended); a landing's refusals go to the mind's buffer and nowhere else (D-UM-8); a raised epoch refuses stale thoughts `Epoch` and a reclaimed seat lands nothing. D-UM-11 and D-UM-47: `IntentSource::Mind` is refused `ORDER`, `RELEASE` and `PLACE` with `Channel` before any other gate and refused `Channel` on the fire channel of a hull with no weapons station; it is never counted on the seat window (D-UM-14); `COCKPIT_CHANNEL_IDS = ["helm", "sensors", "fire"]` with `cockpit_channels()` built from it and `Channel::channel_id` is D-UM-47's written-down table. `set_controller` releases the outgoing controller's rows and due kinds on every transition, which moves the production hash on one reachable path — a pilot's `DRIVE` followed by a `Released` control (`EXIT`, or a linkdead grace running out) — pinned by its own test; the production pins of M1a.1 hold on every other lane. Under a `Linkdead` control a `Mind` hull's discrete intents are refused `linkdead` until the controller changes, as §4's "every other gate applies" says; whether a mind is exempt from the seat-state gate is a lead decision before M4. `rhai` joined the arena library's forbidden substrings. §13's "The snapshot (format 2)" is format 5.
- **§8 D-UM-30, §17 and D-UM-48 (T1, the twin and the divergences)**: as built (17b cut M1a.6, 2026-10-03, `35af96dc`): `Memory` is a canonical `BTreeMap<SmolStr, serde_json::Value>` under `memory_check` (keys 1 to 24 characters, at most 32 keys, values int, bool, null, a string of at most 32 characters or an array of at most 32 ints) encoded as sorted compact JSON so M2's Rhai `mem` encodes to the same bytes (D-UM-30's shape, the stock tier's count 32); `StockMemory`'s fourteen keys over it — §8's seven (`state`, `target`, `last_plot`, `unheld_since_ms`, `quiet_since_ms`, `rallied`, `stand_until_ms`) plus `goal`, `waypoint`, `from_waypoint`, `hold_retry_ms`, `hunt`, `no_line` and `ignore_order` (a Bool; the fixer recommends an Int of the ended order's `expires_ms` before M2 freezes the shape). `NativeStockMind` is `decide` re-expressed arm for arm over `MindView`: `Goal(Cell)`/`Goal(Hold)` on a goal change, arrival off the own cell within `ARRIVE_CELLS` or the `drive_arrived` notice, T0's unreachable give-ups and the lifted `fallback_waypoint` off `drive_no_route`, `Lock` then `Fire` by `engage_orders`' rule, the fire half absent on an unarmed hull (D-UM-48; the unarmed fixture byte-identical at 5,000, 1,000 and 500 ms beside the two-T1 fixture), the emission order `Startup, Goal, Heading, Speed, Lock, Fire` with no `Heading`/`Speed` beside a `Goal(Cell)`; the kernels `best_hostile` (`ContactId` tie-break, aspect as exposure, the held `range_m`), `line_to` the plot cell over `mush_atlas::trace_cells`, the nav pickers, the fallback and the reach readers are free functions both T0 and T1 call; T0's leaky readers are `#[doc(hidden)] pub` for the lane's shadow. The fire rule reads `weapon.ready_in_ms <= lands_ms − view.t_ms` through the new `ThinkJob.lands_ms` (the `MindLand` millisecond; 16b cut M.2's rule, a gun ready at the landing fires) — a departure from the brief's `ready` flag, which alone gave T1 6 of 30 at 15.3 shots a minute; the lead is owed a confirmation. **Gate B** measured against T1 (there is no T2 yet): 30 pooled matches, T1 13, T0 17, draws 0, Wilson 95 per cent [0.274, 0.608] containing a half, beside a T0-against-T0 control pool §17 did not name (red 26, blue 2, draws 2; T1's draws at most the control's plus six is the second assertion); the control is red-biased, so the interval is a weak test of one side. **The divergences**, §17's six re-counted as thirteen instrumented counters on the live arena after each think's pulse, with one more no counter sees: `tiebreak` (the `ObjId` tie-break → `ContactId`) 43; `line` (`reflex_has_line` to the true pose → `line_to` the plot cell) 0; `support` (the charge's own row → `Support { of }` held or none) 0, where a self-charged `Support` reaches T1 as `Support { of: None }` and it stands as a blind supporter while T0 fights on doctrine (unreachable until cut L); `reads_dead` 0 (value-identical by construction, above); `landing` (the think at the lattice millisecond and the landing one substep later, against T0's staggered `ReflexTick`) 10,436, the dominant one; `no_line` 0; `fan` (the hunt fan as a motor rule at the landing) 11; `order_hint` 0 and `order_end` 0 over `ordered_thinks` 0, so upper bounds only (the pool issues no order); `range_round` 16 and `deg_round` 0 (the view at keyframe resolution); `plot_cell` 28 (T1 keys its patrol and its waypoint fallback on the plot cell and its own object number); `fallback` 0 (the give-ups one think later off `drive_no_route`); the first divergence at 101,377 ms; and the fourteenth: a turret-killed T1 hull reads unarmed (no weapon rows) rather than dry and patrols and hunts where T0 withdraws. T1's Hold arms run only under unit tests, because the view carries no match mode and `MindRunner::think` passes `hold: None` (putting the objective on the frame is a `FRAME_VERSION` change; M4 or a ruling). `view_us` p50 4 and p99 16, `think_us` p50 3 and p99 10 over 1,200 sampled thinks on a clone are §18's figures of record until M4's lane.
- **§22 M1a (the gate as met)**: the "six divergences recorded in doc 16 §11" are the fourteen above, recorded in doc 16 §11's M1a.6 entry; "both harness lanes hold their lenses" held at every sub-cut with one re-bless (`frames_per_pulse_mean`, M1a.3); "`GOTO` as a cockpit verb" is `DRIVE`; the cut's record is 17b §3, row M1a.
- **§1 "The leader rides the commander unit", §3 "`issuer` stays the hint-scope unit; the order's author is `by`", §4 "A mind never orders", D-UM-52 and D-UM-53 (the principal)**: as built (17b cut L.1, 2026-10-04, `4d99b788`; the programme lead's note of 2026-10-03 outranking this document's prose where they differ: no Rhai, a production match on `Controller::Reflex`): `deliberation::CharacterRef { Character { id: ObjId }, Persona { squad: SquadId } }`, tacsim's own enum internally tagged by `kind` with `deny_unknown_fields` and no relation of `mush_store::CharacterRef` (the account link), with `persona_name(squad)` = `red lance leader 1`; `Order.by: CharacterRef` required on the wire after `issuer`, read by nothing that steers and never folded (D-UM-52 delivered: `hash.rs` untouched, an author change moves no hash, M1a.1's two pins re-asserted in both sub-cuts), while `issuer` keeps its meaning and its fold. **The leader is a record, not a controller**: `LanceLeader { squad, by }` rows on `MatchRecord.leaders` (`#[serde(default)]`, no `LIVE_FORMAT` bump), one `Persona` minted by `match_::start` per squad `squads_of` deals — every machine squad of the OOC match, which is D-UM-53's "one leader per AI fill squad", a piloted hull being in no squad — found by `leader_of`; `SquadBrief.leader` names it to the lane (never in the prompt: neither the persona's name nor the words `leader`/`persona` reach the user or system turn, and a `Character` leader prints byte for byte what a persona's did) and `order_of` copies it into `by` beside `issuer: None`. **Departure from D-UM-53 and §22 L as written**: the commander unit the leader rides is a `Controller::Reflex` hull in production, not `Controller::Mind` (M1a.5's rule until M4 flips the controller; since L.2 either machine controller, below); `squad.rs`'s head now says the commander is where the leader rides, still the lowest id, still no station and no capability, the leader down with its wreck. **D-UM-52 as built, stricter on both arms than the plan's brief**: `admit_order(w, match_id, unit, order)` is `send_order`'s first step, so the typed path, the reply path and any direct caller meet one wall — `command_of`'s own rule (16c Q38 as 16b cut Y.1 built it) and nothing weaker: a `Character { id }` is admitted only when `cockpit::flying` answers a seat with `driver == id` and `command_of` grants `order.squad` for that unit, as `Squads` holding a roster of that id that `holds(unit)` or as `Wingman` naming that unit; a `Persona { squad }` only when `squad == order.squad`, `issuer` is `None`, the record's `leader_of` equals `by` and the record's roster of that squad `holds(unit)` (so a direct caller cannot aim a lead's or a persona's order outside its squad); everything else is `IntentRefusal::NoCharacter` (`no character entitled to order that squad gave that order`), pinned unreachable from a cockpit (ten typed refusals are the four pinned words; `forward_order` turns an empty reach into `Busy`), and `apply_intent` still refuses `IntentSource::Mind` with `Channel`; a typed `ORDER` carries `by: Character { id: seat.driver }` with its acknowledgement byte-identical. The side's human lead is not the fill's leader (D-UM-53: persona). `on_deliberate` skips a squad with no `leaders` row before its in-flight slot; `on_deliberation_reply` drops a reply whose `by` is not the squad's leader (counted `refused`) and now releases the in-flight slot once on an undecodable payload (a pre-existing leak of one slot in eight, closed). `OrderWatch` gains `by` beside `issuer` with the reader `ArenaRegistry::order_author`; `human_holds` keeps its body (`issuer.is_some()` and not expired) under a true doc: a live order by a character other than the squad's own lance-leader, today exactly a seated pilot's (the rename to a machine word is M4's, with `NotReflex → NotMachine`). `SNAPSHOT_FORMAT` 5 → 6 (`Order.by` on every carried standing order; `af1`'s digest and 393 lines unmoved, its doc putting the standing order outside the pin; a format-5 document refused by name). The lance lane scripts one lead `Character` per side in `by` (`RED_LEAD`/`BLUE_LEAD`) with its issuers unchanged, so W8's span keeps its meaning; the arena lanes order nothing and the deterministic block did not move. Not built: an NPC `ObjKind::Npc` lance-leader object (IC; the corridor's dispatch design), the leader's name on any pilot surface but the lapse, `by` on the `ordered` tape line (16c candidate: the debrief cannot name the orderer).
- **§8 "the deliberation layer's sentence is retired" and D-UM-32 (the lapse in the leader's name), D-UM-15 (the self-charge), D-UM-49 and §22 L (the leader's words and the leader's squad)**: as built (17b cut L.2, 2026-10-04, `915fc415`): `MIND_LINK_DROPPED` is deleted with no reader left, and `say_expired_orders` speaks from `OrderWatch.by` through `lapse_line(squad, whose, objective)` = `Squad {squad}: {whose} {objective} order has run out; back on doctrine.`: a `Persona` watch says `Squad red-1: red lance leader 1's hold order has run out; back on doctrine.` to every pilot of the squad's own side and the same line to staff behind `Arena match {id}: `; a `Character` watch says `Squad red-1: your hold order has run out; back on doctrine.` to the issuing pilot alone and the character's name to staff (`its leader's` once the object is gone); nobody else is told, each watch once — §8's "in the words a human leader's would" and D-UM-32's retired sentence delivered, and a declared narrowing of 16b cut N.2's sentence, which told the whole side of a pilot's lapse where 16c Q38's no-radio rule never told them of the order. `@arena/status` prints `  orders: N by pilots, M by lance leaders` and `  leaders: 2 sim personas[, red-1 Ada]` or `  leaders: none`, staff-only. **Departure from §22 L's and D-UM-53's "stays `Controller::Mind`", and the lead's one exception to M1a.5's rule that a mind joins no squad**: `Controller::is_machine` (`Reflex` or `Mind`) decides squad membership at both `squads_of` callers through the one builder `CustodyUnit::squad_input`, so a Mind hull is dealt into a squad under a leader and an `ORDER` reaches it through the unchanged path (D-UM-15), while the engine reads **no** mind memory (D-UM-49: a sealed author's `mem` is never relayed into another principal's provider call): a Mind member's signal state is the `Startup` placeholder until M4 revisits the signal for minds, so a persona's brief for a Mind squad carries no `doing:` histogram beside members all at startup — a T0-versus-T1 divergence in what the leader is told, reachable in the L.2 fixtures alone since production fields no Mind hull before M4. **D-UM-15 amended for the self-charge**: a `Support { of }` naming the hull itself re-keys for a Mind hull to `MindObjective::Engage { target }` through the Engage arm's own resolution (`preferred_contact()`, which is `Order.target` for a Support, in the issuer's or the commander's scope, then the hull's own held row or none) where M1a.6 wrote `Support { of: None }` and T1 stood as a blind supporter; T0's `ordered_target` falls through to the same hint, so both tiers charge, pinned by the approach, the lock and the first fired line where HEAD's blind supporter stood on. Two T0-versus-T1 divergences named and not changed: a self-charged Mind hull's `order_expired` journals `engage` (`expire_mind_order` journals the held objective's word) where a reflex hull's expiry and both hulls' `ordered` line say `support`; and a reflex hull's frame-time `frame_order` under a self-charge reads `Engage` with no reader today. §17's twin: `shadow` gains the `(Support { of }, Engage)` arm so its `support` counter still sees a self-charge, the figures unchanged in release (T1 13 of 30, [0.274, 0.608], `support=0` over `ordered_thinks=0`; the arm has never run). The mandatory two-T1 cross-tempo fixture holds three Mind hulls as one squad under a persona-led `Engage` at 5,000, 1,000 and 500 ms with one event list, one hash sequence and each hull re-keyed to its own held row or none (M1a's residual (2), an unclassified held row, pinned neither way). The watch stays per squad and latest-wins (16c Q43), so a pilot whose watch a teammate's later `ORDER` overwrote hears no lapse at all (declared in doc 16 §11).
- **§22 L (the gate as met)**: the deliberation fixtures pass with `by` present and `issuer` unchanged and the hint resolving in the same scope; a forged reply with no `by` is undecodable, issues nothing and gives its slot back, and `IntentSource::Mind` is refused `ORDER` with `Channel` as before; a wrecked leader's hull raises no signal (pinned by a wreck set before pulse 0, not a mid-match wreck: 17b row L's residual); the harness with the LLM disabled is unchanged and its deterministic block did not move; doc 16's W7 row, §6 paragraphs and §11 were amended by the cut's record; the cut's record is 17b §3, row L.
- **External review of 2026-10-04 after review (seven findings, all confirmed by nine read-only
verifiers; the dispositions in the programme's plan-data under `review-2026-10-04/`)**: amended
before any code, so no cut builds against the old text — D-UM-23 and §6 (a snapshot carries the
think's answer, never a recompute; M3, with M4's pool awaiting a batch before it encodes), D-UM-17
and §5 (the floor checked at every arming and cited fire, `Unlawful`, the `Lock` target
classified-only; M1b, the seal capturing the floor in M3, the governed-bit exemption in M8), D-UM-20
and §6 (a landing's same-millisecond impact applied before that millisecond's thinks; M1b's first
sub-cut), D-UM-4 and §3 (`MindView.scan_tables`; M1b), D-UM-25 and D-UM-30 and §7 (data-size limits
as a backstop sized to twice Class III's largest valid memory, ASCII memory strings; M2), D-UM-40,
D-UM-49 and §10 (PendingTransfer; M7), and the proposed D-UM-56 and D-UM-57 with Q-UM-17 and Q-UM-18
(private filing, the owner's licence, dependent forks; M7, pending the owner). Two of the findings
are M1a omissions recorded in the 17b M1a row: as first built, `run_mind_events` drained landings
and thinks in one pass, so a zero-flight `Impact` a landing scheduled at its own millisecond was
applied after that millisecond's thinks (reachable whenever two minds' stagger offsets differ by the
substep length; `laser_mk1` is a shipped beam); and `MindView` omitted the keyframe's `scan_tables`,
leaving `ViewContact.condition.table` an index without its table.
- **§6 and D-UM-20 as amended (the order of a millisecond), §3 and D-UM-4 (`MindView.scan_tables`)**: as built (17b cut M1b.0, 2026-10-04, `0c156218`): the due loop still holds a millisecond's `MindThink`s and `MindLand`s while the millisecond's other due events pop; when the heap's head has left the millisecond, every held landing runs (`Arena::run_mind_landings`, by unit then `think_seq`, `ActionHash::Pending` opened before each `mind_land`), the loop goes round so whatever those landings scheduled at that millisecond pops next — a beam's zero-flight `Impact` (the case the order exists for), a goal's `MotorTick` when the landing falls on the substep's end, a `StartupComplete` under a `startup_ms` of nought — and only then the held thinks (`Arena::run_mind_thinks`, by unit then `think_seq`, opening no action hash because a think journals nothing); `run_mind_events`' one pass is gone. So D-UM-20's "landings, then the heap's events at that millisecond, then the thinks" reads precisely: the millisecond's already-queued events, then its landings, then what the landings scheduled at it, then its thinks. Two minds' beams at one millisecond are mutual — both `fired`, both `impact`, neither hull a wreck when it lands — exactly as two `ReflexTick`s are. **Declared T0-versus-T1 asymmetry**: a `ReflexTick`'s beam `Impact` at a millisecond pops before that millisecond's held mind landings, so a reflex beam wrecks a mind hull before it lands (and `mind_land` drops a wreck's landing silently), while a mind's beam never pre-empts a reflex tick at the same millisecond either, the tick having already run: reflex ticks and mind landings are not mutual, reachable only with laser hulls on both controllers at one millisecond; M4's flip retires it. Two further mind-only reorders against M1a.5 are declared and pinned by no fixture: a landing's `MotorTick` on a substep end now runs before that millisecond's thinks, and a hull wrecked by a same-millisecond landing's beam no longer thinks at that millisecond. `MindView.scan_tables: Vec<Vec<SmolStr>>` is the keyframe's `scan_tables` moved verbatim (a per-view index, never stable across thinks), `m1a5_the_view_is_the_keyframe` asserts the key and a mixed-chassis fixture holds two tables; no `FRAME_VERSION`, hash or snapshot moved. The production LINE-hash pin was recorded on `e23694b0`'s bytes before any edit, in three halves (`M1B0_SEAT_LINES`, 28 lines of the (h) seat script at three cadences; `M1B0_DUEL_LINES`, 30 lines of the shipped duel, which holds no shot in its 60 s; `M1B0_J_LINES`, 58 lines of the (j) field with seven reflex shots), and every later sub-cut re-asserts it beside the three substep pins; a line pin cannot see a trigger that sets `Pending` and journals nothing, which M1b.1 pins in-source per trigger. This entry supersedes the M1a.5 entry's "run last, landings before thinks"; M1a.5's "a think reads a notice staged earlier in its own millisecond" stands.
- **§5 and D-UM-16, D-UM-17, D-UM-14, D-UM-19 (seat slots, the arming law, the floor and every carried shape)**: as built (17b cut M1b.1, 2026-10-04, `137e49c7`; `SNAPSHOT_FORMAT` 6 → 7 once, the `af1` pin 393 → 396 lines for `state.conditions`, `state.held_said` and `state.odds_floor_permille`), the programme lead's note outranking this document's prose: (a) slots live in a new per-hull `Arena.conditions: ConditionTable` for a hull of ANY controller, not in `MindState.conditions` (a piloted hull has no mind row and `release_mind` deletes the row on every transition): stable 1-based numbers (a re-arm of the same weapon keeps its number and replaces the terms; a disarm leaves the other's), `STOCK_CONDITION_SLOTS = 2` until M3's class table, cleared on every controller transition, the wreck retain and per weapon at `RemoveWeapons`; (b) `FireCondition` gains `min_range_m` beyond D-UM-16's list so `range > n m` fits; (c) the target rule: **a slot never takes a lock** (Q-M5) — `Lock`, the rule language's default, fires the hull's lock when held and classified and is `Idle` on an unclassified or absent one; `BestHostile` the lock when classified of another side and not reading dead, with no arena-side ranking (§5's "ranges over classified contacts" narrowed to the lock until M5); `Contact { id }` is refused `Unlawful` at every arming whatever the id in M1b, so a held and an unheld id answer one word (§5's "must be classified when evaluated, else idle" is kept for `Lock`); a `Lock`-target slot keeps firing at a lock whose row reads dead until the track expires (about 3 s), a ruling owed; (d) the warrant is `warrant(unit, slot, at_ms)` in a fixed term order — the target, rounds above `min_rounds_left`, the ROUNDED range inside the bounds, the aspect in the mask, `resolve_seen`'s odds at or above `min_permille`, the first failing term a `HeldWhy` — with no readiness and no empty-magazine term: a gun not ready or dry is `Idle`, and a cited fire on a cycling gun reads `Cycling`, never `Unwarranted`; (e) evaluation at six triggers, §5's three widened: the weapon's `WeaponReady` (that weapon's slot alone), the hull's own `SensorPass` and a received `Share` that refreshed the lock's row (reading `last_seen_ms`/`own_seen` and the answered share pairs, the plot only), the `StartupComplete` flip, a mind's landing ONCE after every intent of it (never per landed `Arm`, which would read the lock the think just dropped), and for a seat after a `LOCK` or an arming resolved in phase 4 — never once per pulse — firing `Fire { weapon, slot }` through `apply_intent` under a new `IntentSource::Cond` (never on the seat window; exempt from the discrete branch's `Linkdead` gate, declared and pinned) and `resolve_fire_by(.., FiredBy::Cond { slot })` at that millisecond, the `fired` line gaining `by: cond` and `slot` (D-UM-14 delivered); a slot's own resolver refusal is dropped silently, so a slot held for heat waits for the next pass or `WeaponReady`; a hull with no slot costs one map miss per trigger and no action-hash touch, and `share_pass` collects a receiver pair only for a hull holding a slot, so a production share allocates nothing; (f) `IntentKind::Condition(Arming::{Arm, Disarm})` is a DISCRETE order on the fire channel, word `fire`, queued at phase 2 through the seat window and resolved at the pulse's end in phase 4 IN ARRIVAL ORDER with `LOCK` and `FIRE` through `Arena::resolve_arming` (`Unknown`, `Wreck`, `NoWeapon`, `Unlawful`, then `NoSlot` — the law before the count; a disarm or a re-arm drops the slot's `held_said` rows), a mind's at its landing in emission order — decided nowhere in this document and declared here: a continuous phase-2 arming would have fired at the old lock for a pulse of shots when `LOCK BA` preceded it in the same pulse, which the zero-`dt` cross-tempo replay cannot see, so a non-zero-`dt` fixture pins the typed order; (g) three new `Refusal` words, not D-UM-17's two, each with one sentence whose home is `CockpitError` so the cockpit's pre-check and the arena's landing refusal print one string: `unwarranted` `That has no warrant.`, `unlawful` `The Compact floor is 10%; a condition may not ask for less.` through a third `RefusalDetail::Floor { permille }` (a `Contact` target's `Unlawful` carries no floor and says `The Compact floor forbids that condition.`), `no_slot` `Every slot is armed; FIRE <weapon> OFF frees one.`; (h) the floor is `[tacsim.minds].odds_floor_permille` (seed 100, validated 10 to 100 in steps of 10 until M3's seal — Q-M4, so `{floor/10}%` is exact and Line's 100 is lawful by construction) captured into `Arena.odds_floor_permille` at check-out through `spawn_arena`'s new argument from `lobby::minds_config`, carried by the snapshot and NEVER folded into `state_hash` (a per-match constant; folding it would move the production hash for a value no production match reads before M4), so a resumed arena keeps the floor it was checked out with and a config reload mid-match moves nothing — the capture pinned across a resume; (i) `hold_fire` is enforced in `resolve_fire_by` (D-UM-19 named `resolve_fire`, kept as the `Seat` wrapper) as `Unwarranted` for `Cond` here and for `Mind` in M1b.2, reading the ARENA's standing order (a Mind hull's `order.hold_fire`, a Reflex hull's `constraints.hold_fire`, a Pilot hull never), with `Seat` and `Machine` never gated; (j) `StockCard` (`arena/card.rs`, §11's terms in `ReflexConfig`'s units where mapped, `line(cfg)` the anchor) sits on `MindState.card` with `MindState::fresh(&ReflexConfig)` and `ThinkJob.card`, hashed inside item 3c after `strikes`; hash items 3d (slots) and 3e (`held_said`) fold per row with no count prefix, so a hull with no slot folds nothing new and the production hash did not move by substep or by line; (k) the two notice kinds `slot_fired`/`slot_held` and `RefusalDetail::Floor` are defined and carried here and staged by nothing until M1b.4. The release-mode evaluator cost on a 300-hull flat field: p99 7 µs a call at `WeaponReady` and at a pass, p99 276 µs for a 150-receiver share, report-only (17b §2).
- **§5 and D-UM-17, D-UM-19 (the law for minds), §11 and D-UM-43 (T1 arms its slots), §17 (the divergences)**: as built (17b cut M1b.2, 2026-10-04, `cf0a78bd`): `resolve_fire_by` gates `FiredBy::Mind { slot }` after the `Cycling` and `Empty` tests and before `Overheated` and `PowerNotOnline`, in the pinned order — the cited slot exists and names the weapon (else `Unwarranted`), `condition_is_lawful` against the arena's floor (`Unlawful`), hold-fire off the arena's standing order (`Unwarranted`; Q-M7: the resolver reads the arena's row, so a T1 that ended a hold-fire order early is silenced for the order's TTL — `hold_after_end`, declared and fixtured, not counted), the warrant (`Unwarranted`; an uncited `Fire { slot: None }` likewise). T1 arms at think time, in a think whose decision reaches `engage_orders` with its gates passed (power online, not under `hold_fire`, not dry): one `BestHostile` slot per weapon for its first `STOCK_CONDITION_SLOTS` weapons in component order WHATEVER THE HEAT (the resolver's `Overheated` holds a hot slot's shot and the slot stays armed; a `would_overheat` skip would put T1's slot numbers out of step with the arena's) at its card's fire discipline — `min_permille` the card's `min_odds`, the aspect mask, `reserve_pct` of `rounds_max` as `min_rounds_left`, `max_range_m` the ceiling that admitted the row capped by the weapon's reach — else one `Disarm { weapon: None }`, re-emitted every think here and only on change from M1b.3; the emission rank gains `Condition` after `Startup` and before `Goal`; a citation `Fire { weapon, slot }` only in a think that also plans a `Lock`, with T0's first-ready-weapon rule (`would_overheat` kept) for that shot; a third weapon is never armed nor cited. The four M1a.5 mind fixtures and M1b.0's `beam_shot` were re-scripted to arm and cite with their `fired` milliseconds kept. **T0-versus-T1 divergences** counted by the twin probe beside M1a.6's thirteen: `cond_fire` 2,811 (a T1 `fired` line `by: cond` at a `WeaponReady`, pass, share, landing, arming or startup millisecond where T0 fires on its tick) with its sub-count `cond_stale` 9 (a cond fire at a lock the latest planned `Lock` has left); `unwarranted` 0; `unlawful` 0, ASSERTED nought on both stock lanes (Line's 100 per mille is lawful against any floor the bound allows; a non-zero count is a stop); `floor_held` — T0's shadow planned a fire the T1 slot's warrant answered `Why(Odds)`: the shipped table's `p_min 10` puts a 2 km shot at about 6 per cent, T0 fires it and Line at the floor holds, so **D-UM-43's "Line on its defaults is today's machine" is true except at the floor D-UM-17 forbids T1 to go under** — 0 on the pool (it fights inside the 600 m sight) and 156 on a probe field that presents nothing, a lower bound; `slot_starved` 0; and a seventh beyond the plan's six, **`slot_parallel` 449**: each armed slot fires its own gun at its own `WeaponReady` while T0's `engage_orders` fires one weapon a tick, so on the two-gun tank the coaxial gun is no longer serialised behind the main gun and T1 fires 30.491 shots a minute against T0's 24.977 (about 25.8 without the 449) — a second D-UM-43 exception, named and counted, not retuned, a lead ruling owed. The twin moved by the timing alone, 13 → 15 of 30 and Wilson [0.274, 0.608] → [0.332, 0.668], the interval still containing a half (17b §2); six fixtures' blues gained a presenting profile, carrying hits where they had misses.
- **§3 and D-UM-4, D-UM-5 (the frame grows: the arming and the floor; Q2), D-UM-15 (the classified re-key), §5 (`WEAPONSTATUS`'s arming)**: as built (17b cut M1b.3, 2026-10-05, `189aff85`; `FRAME_VERSION` 2 → 3, a wire version: a deploy needs the gateway restart and a fresh `web/dist`): `FrameWeapon.armed: Option<FrameArming { slot, target, min_permille, aspects, keeps, min_range_m, max_range_m }>` compared in `same_as` and filled from `Arena.conditions` by weapon id under the stable number; `Frame.odds_floor_permille` (a per-match constant, never compared) and `MindView.odds_floor_permille`, so a mind learns the floor lawfully (D-UM-4); the keyframe's per-weapon `armed` object (`null`, or its slot and terms with `target` the KIND `lock`/`best`/`contact` and never a designator) and the top-level `odds_floor_permille`; ONE pure `arming_words(&FrameArming)` (`S1: >=60% side/rear keeps 8`, `S1: best >=10% under 420m`) printed on its own line under an armed weapon by the TUI pane (wrapping past 46 by the pane's rule), the web's rows and, from M1b.4, line mode — the departure from §5's bracket tail, for the 120-column floor; `REFUSAL_WORDS` gains the three words in both rings; the four TUI goldens and three web fixtures re-blessed. **Q2 (M1a's residual (1)) ruled and built**: `frame_odds` seats the hull's lock first when `Classified` of another side and still in the match, then every other such row nearest-first by plot range with `ContactId` ascending, the first `STOCK_ODDS_ROWS`, never the hull's own side and never an unclassified row — a narrowing of §3's "against any classified contact" and D-UM-5's "classified contacts only": a friendly row and a friendly lock print the dash (the `LOCK` line unchanged); the hand-built fixtures were edited, the proof runs on a built arena and the help's Seeing paragraph says what a dash is. **M1a's residual (2) ruled and built**: `rekey_order` keeps a row only when `held_classified`, through its one closure (the Engage arm and L.2's self-charge) and the `Support { of }` arm; `MindObjective` carries a target on `Engage` only. **Declared T0-versus-T1 divergence `order_hint_late`**: `rekey_order` runs once at the order's landing, so an `ENGAGE` landing before the mind's hull classifies its target leaves T1 on `Engage { target: None }` for the TTL where T0's `ordered_target` re-resolves every tick; a late re-key would need the hint's object kept on the mind row, a 16c candidate. T1 emits an `Arm` only for a first-two row whose view `armed` is absent or differs from what it wants (the slot number not compared) and a `Disarm` only beside an armed row, and cites `armed.slot` off the view, `i + 1` only for a weapon armed in the same think; `frames_per_pulse_mean` did not move; fan-out `panel_bytes_max` 7,931 of 16,384. `FrameArming.target` is `SlotTarget`, so a mind's view would carry a `Contact` target's id the keyframe writes as `contact` — unreachable while every `Contact` target is unlawful, but M2 freezes the view with that asymmetry.
- **§5 and D-UM-18 (the pilot's surface), D-UM-17 (the floor refused as typed), §4 (the station verbs)**: as built (17b cut M1b.4, 2026-10-05, `69880812`): `FIRE <weapon> WHEN <condition> [AND <condition>]…` (at most four, no `OR`) over exactly the eight slot-fitting terms of D-UM-18's language — `odds [weapon] >= n` (1 to 100; `10n` under the frame's captured floor refused as typed through `fire_refused` as `FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less.`, the arena's `Unlawful` at the landing the byte-identical backstop; a rule with no odds term compiles to the floor; no upper refusal, 100 simply never holds under `p_max 950`), `aspect = front|side|rear` (a comma list), `range < | > n m` (against the ROUNDED range; inclusive at the warrant while the reply says `under`/`past`, a ruling owed), `band = close|effective|far` (the fixed metres `contact_band` cuts for that weapon on that frame), `rounds > n | n%`, `reserve` (`rounds > 0` on a piloted hull), `target = identified` and `lock` (no-ops); everything else is the ONE outside refusal `That condition is not one this core takes; help cockpit lists the eight it does.`; the think-evaluated terms (`hit within`, `hostiles >=`, `order =`, `target = moving|stopped|hurt`, the section readings, `own`, `heat`, `no lock`) and the `LOCK …` rule actions are not built (M2 and after). **`HOLD FIRE <weapon|ALL>` is spelled `FIRE <weapon|ALL> OFF`** (Q-M1), a plain disarm with two unconditional replies (`Cleared: cannon fires only when you say.`, `Cleared: your guns fire only when you say.`); the conditional veto is M5's. The English is written back with NO slot number (`Arming cannon: fires your lock when your chance to hit is 60% or better and it shows its side or rear.` — the number is the arena's to give at phase 4), the arming is a discrete order through the seat window with no power pre-check (a slot may be armed cold), the count is pre-checked off the mirror and told by the arena as `NoSlot` when two armings race a pulse, and every pre-existing `FIRE` line, reply and refusal is byte-identical. `WEAPONSTATUS` prints the arming line indented under the weapon's row (`  S1: >=60% side/rear`; `WeaponRow::render` unchanged, the widest shipped row and the widest arming line pinned under 120 columns). The sentences name the SLOT, not a rule number: `Cannon fires at BA by S1 (62%, side).` (Q-M2) staged on the shooter's hull at every slot shot and never for a cited mind `FIRE`, and `S1 held the cannon: {54% under your 60% | BA shows its front | BA at 650 m, outside 200 to 600 | 7 rounds, keeps 8}.` at most once per slot and contact per `HELD_SAID_MS` 10 s through `held_said`, never for an idle slot; the precedence in the four lines `help cockpit` prints (`An ORDER's HOLD FIRE beats everything.`, `FIRE <weapon> OFF clears a slot; a cleared weapon fires only when you say.`, `One slot a weapon: a second FIRE <weapon> WHEN replaces the first.`, `Rules beat sliders.`) — §5's "first matching rule wins per channel" has no work to do while a weapon holds one slot. The bot's reader learns `  S<n>: …` as `Line::WeaponArming`, which it never acts on, and its picture keeps an open `WEAPONSTATUS` reply across it — a reader-contract delta recorded in doc 16 §11's AM entry.
- **§11 and D-UM-43, D-UM-44 (the card's setters and the presets), §8 (the memory shape), §14 (`@mind/set`, the owner's hand on a T1 card), §22 M1b (the fill controller and the pooled measure)**: as built (17b cut M1b.5, 2026-10-05, `d2d49389`): `StockCard::set(slider, value)` refuses every range, step and choice by name through `CardError { Range, Halves, Choice, NoSuchSlider }` (refuse-not-clamp; `fight_home_pct` in halves; `min_odds` 10 to 95 step 5 with the floor the caller's); `preset(name, cfg)` over `line(cfg)` for the six presets exactly as §11 lists them; `name_of` (`Line, edited (min_odds 60)` against the nearest preset by count); `read_back` the five section lines with the eleven unread sliders marked `[M2]` (Q-M3: carried, read by nothing; ten marks, the screen's two sharing one) and `band 70%` alone, the fire-discipline line inert with its reason on an unarmed hull; T1 reads the mapped sliders through `as_reflex` (the overlay over the arena's `ReflexConfig`: `engage_band`, `withdraw`, `stand_min`, `search`, `hunt_after`, `rally` with 0 → never, the weight triple by `prefer`; everything else the base's), with the regression anchor read as Q-M6's three proofs — `line(cfg).as_reflex(cfg) == cfg` for the default and a non-default doctrine, the two-T1 fixture's events and intents recorded before the overlay edit and asserted after it (its hash constants retired after the rename), and the twin equal to M1b.4's line for line. `set` validates sliders independently (`break_off_pct 10` on Line leaves `stand_min` 175 above `withdraw` 100, a pair `ReflexConfig::validate` refuses): §11 asks for no cross-slider rule, a card-only state for the lead or M2. **`@mind/set <unit>[=<preset> | =<slider> <value>]`** is one staff verb `@mind` gated as `@arena` (not a pilot's `MIND …`, which is M5's): it validates by name against the mirror's known card (else `line`), pre-checks `min_odds` against the MATCH's captured floor on `MatchMirror.odds_floor_permille` (written at `open_match` and at boot resume, never the live config; pinned with the live block at 100 and a match at 50 across a reboot), refuses `is not flown by a mind` off the mirror's controllers, and offers `ArenaMsg::MindCard` into the match's inbox; the arena stages it in arrival order and applies it in phase 2 TO ITS OWN ROW under its own `condition_is_lawful` check over the card's `min_odds` against `Arena.odds_floor_permille` (vacuous under M1b's bound, written for M3's wider floor), journalling `mind_card` with the whole card or `mind_card_refused` (`not_mind`, `unlawful`, a `CardError`'s text), and the next think reads it (one event list, one hash sequence and one think sequence at three cadences); the read-back is written from the tape (`as checked out`, `on the tape at N ms`, `sent at N ms, not yet on the tape`, `not read since the resume`). **§8's memory shape**: `ignore_order: bool` → `ignore_until_ms: Option<i64>` holding the ended order's `expires_ms`, so a newer order is obeyed at once (the M1a.6 fixer's recommendation, taken before M2 freezes the shape; `hold_after_end` asserts the key). **`[tacsim.minds].fill_controller = reflex | mind`** (default `reflex`) is a lab knob on the uncarried block, read once by `match_::start` from the lobby's reader (and by `@arena/start`), not a lobby choice (`SIM AI MIND` is M5's): with `mind` every unseated hull is checked out as `Controller::Mind`, dealt into squads by L.2's `is_machine` and flown by T1 with its slots armed, the board's `flown by doctrine` unchanged and pinned. The bot's `#[ignore]` lane flew the 30-match pooled measure twice — `MIND RATE: 16 of 30 for the bot against a conditioned T1 (red 8 of 15, blue 8 of 15, draws 0, timeouts 0)`, Wilson 36 to 70 per cent, in parallel mode, and 15 of 30 (red 6, blue 9), Wilson 33 to 67, sequentially at the gate — every match decided in 33 to 78 s; the lane's per-match kill figures are the bot's own picture's and read mostly 0 though every match was decided, not reconciled with the arena's record; `ci.yml`'s manual bot job no longer flies the lane, which runs by hand.
- **§22 M1b (the gate as met)**: a conditioned shot resolving on the `WeaponReady` millisecond and not a pulse later — `m1b1_a_conditioned_shot_resolves_on_the_weapon_ready_millisecond_at_three_cadences` (the `Arming` trigger at 5,000 and the `WeaponReady` shot at 9,000, mid-pulse at the 5,000 ms cadence, one event list and one hash sequence at 5,000, 1,000 and 500 ms and on the task's wakes); `FIRE cannon WHEN odds >= 5` refused `Unlawful` as typed — `m1b4_fire_when_under_the_floor_is_refused_unlawful_as_typed_and_at_the_landing` (nothing forwarded; the arena's landing the same bytes); a beam landed at a think's millisecond seen by that think at three cadences — `m1b0_the_beam_before_the_think_holds_at_three_cadences`, with mutual beams both landing; the line-mode transcript of `FIRE cannon WHEN odds >= 60 AND aspect = side,rear` firing only on flanks while a typed `FIRE` still fires — `sim_weapons::m1b4_a_conditioned_cannon_fires_only_on_flanks_while_a_typed_fire_still_fires`; the 30-match pooled measure with Wilson intervals — the lane's two `MIND RATE` lines; the harness lenses — held at every sub-cut with the deterministic block equal on 109 fields and no re-bless (`frames_per_pulse_mean` did not move under Q2); the Line card as T1 at HEAD — Q-M6's three proofs and the twin equal across M1b.3, M1b.4 and M1b.5 with `unlawful` 0; the stock card on an unarmed hull with its fire-discipline section inert and no slot — `m1b5_an_unarmed_hull_takes_any_preset_with_its_fire_section_inert` (no seal exists before M3: nothing is sealed and nothing refuses); 16c's `Q45` row marked landed; the cut's record is 17b §3, row M1b.
- **§7 "The crates", "The engine", "Limits", "The budget", "Parameters", "Lints", §8 "Memory" and "Faults", and D-UM-1, D-UM-24, D-UM-25, D-UM-26, D-UM-30, D-UM-31 (the sandbox alone)**: as built (17b cut M2.1, 2026-10-05, `dec5328b`), on the programme lead's note for M2 that outranks this document's prose where they differ and never its decisions without saying so — the Rhai runtime runs SYNCHRONOUSLY at the think point as `NativeStockMind` does; no pool (M4's, D-UM-22), no seal, no core, no licence, no `MindFault`/`MindDropped` ledger kind (M3's), no mind on any production path, `FRAME_VERSION` 3 and `SNAPSHOT_FORMAT` 7 held: `crates/mush-mind` depends on `rhai` alone, pinned `=1.26.0` at the workspace with `RHAI_ABI` tested against `Cargo.lock` and the manifest line, and denies all SIX restriction lints — `expect_used` beside the five §7's Lints paragraph listed, because the restriction-lints census reads a zero in a crate's row as a promise to deny (the census 43 → 44, every tally plus one, no panic-budget file for a ceiling crate at zero). `Sandbox::new(limits, api)` is `Engine::new_raw()` plus exactly D-UM-24's seven packages (never Math, Time, MoreString or Fn), `DummyModuleResolver`, `eval`/`import`/`export`/`Fn`/`call`/`curry` disabled through `disable_symbol`. **Departure from §7's reading of `disable_symbol`**: it acts on reserved keywords and literal tokens only and cannot touch an identifier, and `LanguageCorePackage` carries `sleep(INT)`/`sleep(FLOAT)` and `exit()`, so `sleep` is SHADOWED by two engine-registered functions raising an uncatchable refusal (the engine's own namespace is `global_modules[0]`, searched before the packages; asserted in under 100 ms) and `exit()` is ruled a RETURN, not a fault — the buffered intents land and the memory is what the script wrote, pinned (`call_fn_with_options` maps `ERR::Exit` to `Ok`); `parse_json` and the reflection functions stay (a FLOAT they yield faults at the boundary). The limits as D-UM-25 amended — call levels 12, expression depths 48/24, variables 64, functions 32, modules 0, strict variables, `fail_on_invalid_map_property`, anonymous functions off, the data-size backstop 256/8,192/8,192 asserted at twice the largest class's 128 keys, optimisation `Simple` — with the function cap a PARSE-TIME cap (`ErrorTooManyFunctions` does not exist in rhai 1.26.0; a 33rd function is `CompileError::Parse`, never a runtime `limit`); `max_operations` is the caller's backstop and `EngineLimits::default()` is class I's 1,500 (one engine serves every class; the larger-class backstop is M3's with `CoreProfile`). **The budget (D-UM-26) keeps its own count**: a package method's callback (`map`, `filter`, `sort`, `reduce`, `retain` and the rest, reachable by naming a script function) runs on a CLONED runtime state whose operation count restarts from the caller's and is thrown away, so trusting the number `on_progress` is handed would let a think spend unbounded operations in sibling callbacks; every `on_progress` call is exactly one operation, so `budget::check` counts its calls and ignores the argument, answers the cause word `ops` the first time `ops + fee > limit` and on every operation after, and `Sandbox::think` re-checks once after the call so a fee charged by the last call trips (`budget::tripped`); no clock anywhere, the trip at the same operation on every host. §7's "native integer helpers `abs`, `min`, `max`" are the Arithmetic and Logic packages' own; `clamp` (lo > hi refused) and `isqrt` (a negative refused) are registered natively; `sin_m`, `cos_m`, `bearing_to`, `range_to` and `arc` are the game's (M2.3). `on_print`/`on_debug` write a thread-local trace of at most `TRACE_LINES` 8 lines of `TRACE_WIDTH` 120 characters with every control character spaced, answered beside every result and never journalled. **Memory (D-UM-30) is bound as `this`, not "`mem` in scope"**: `Sandbox::think` decodes the bytes into a `rhai::Map` (empty or undecodable → the empty map) and binds it through `bind_this_ptr` to `fn think(view, card)`, read back after the call; `mush_mind::memory` is the ONE canonical codec — `Value::{Int, Bool, Unit, Str, Ints}`, `Memory = BTreeMap<String, Value>`, `Limits { keys, key_chars, string_bytes, array_len }`, `check` (the count first, then keys and values under the printable-ASCII rule `0x20..=0x7E`), `encode` as sorted compact JSON with only quote and backslash escaped, hand-written with no serde and byte-identical to M1a.6's `serde_json` for the same shapes (pinned by M2.2), `decode` the exact inverse refusing whitespace, floats, exponents, nesting, any other escape, raw bytes outside the range, leading zeros, `-0`, i64 overflow, unsorted or duplicate keys, trailing bytes and empty input, so only bytes `encode` could write are accepted; `from_map` reads a FLOAT anywhere (arrays and nested maps included) as `MemFault::Float(key)`, anything else off the five shapes as `Shape`. **Faults (D-UM-31) as built in the crate**: `Fault::{Ops, Limit, Type(String), Memory(MemError), Panic}` with `cause()` the words `ops`/`limit`/`type`/`memory`/`panic`, classified on the INNERMOST error (`unwrap_inner`) so a termination, a limit, an exit or a refusal inside a callback keeps its meaning through `ErrorInFunctionCall`'s wrapping (`ErrorTerminated` → `Ops`; `ErrorTooManyOperations`, `TooManyVariables`, `TooManyModules`, `StackOverflow`, `DataTooLarge` → `Limit`; anything else → `Type`, one bounded line); `system_error(text)` is the ONE constructor of a refusal — an `ErrorSystem` uncatchable by `try`, passed through a script function unwrapped — and it LATCHES the think's first refusal in a thread-local that `Sandbox::think` clears at its start and reads ahead of every end but a panic, on the `Ok` path too, so a refusal a package callback wrapped and a script caught still ends its think; every refusal renders `<text>: refused`; a panic under `catch_unwind` → `Panic`; a replaced `this` → `Type("this is no longer a map")`. The AST cache (`Sandbox::compile(source, Digest, abi)`) keys `(Digest, &'static str)`, requires `fn think` of exactly two parameters, clears the top-level statements (a refused source is never cached) and never evicts (M3's seal store). **The scope contract** (the reviews' F1, pinned by a test): under strict variables a function body cannot read a `call_fn` scope variable (a compile error), every `fn` body is parsed fresh and sees no top-level `const`, a script-to-script call runs on an empty scope, and `compile_with_scope` would fold a scope constant into the AST cached once per `(digest, abi)` — so `Think` carries NO constants and no scope, and the per-think facts are the game's native zero-argument functions (M2.3), which `Simple` never folds and which resolve inside a helper on the cached AST, one call per think. Two readings scripts live under: no string + INT (`"k" + i.to_string()`), and a bare script-function name is still a function pointer for the packages' callback methods (charged and latched; removing it would shadow those methods, a lead question).
- **§7 "Randomness" and D-UM-29 (the sixth label), §8 "Faults" and D-UM-31, D-UM-32 (strikes and the drop as a rule), §3 and D-UM-1 (the job and the result; the definition behind an `Arc`), §5 and D-UM-17 (ruling (a)), §11 and D-UM-43, D-UM-44 (ruling (b)), §5 and D-UM-18 (ruling (c)), §17 (the kernels)**: as built (17b cut M2.2, 2026-10-05, `fb703386`; the arena's side of the seam prepared against T1 alone before any Rhai enters tacsim; `crates/tacsim` depends on `mush-mind`, `composition_edges` 31 → 32): `arena/mind.rs` re-exports `mush_mind::memory::{Memory, Value as MemValue, MemError}` as the one codec with `STOCK_MEMORY_LIMITS { keys 32, key_chars 24, string_bytes 32, array_len 32 }` and the ASCII rule (D-UM-30 as amended; it changes nothing T1 writes), `memory_check`/`encode_memory`/`decode_memory` thin over it, `StockMemory`'s fourteen keys over `MemValue` (`Int` for every integer and `target`'s bit-cast, `Ints` for `goal`, `hunt` and `last_plot`, `Bool`, `Unit` for an absent field), byte-identical to the two-T1 and unarmed fixtures' memories recorded on `dec5328b` before the edit (100 per-think digests, four whole memories) and to `serde_json::to_vec` over 10,000 random valid maps. `ThinkJob` gains `seed: u64`, `dropped: bool`, `view: &Arc<MindView>` and `map: &Arc<ArenaDefinition>` — **`Arena.definition` is `Arc<ArenaDefinition>`**, wrapped once in `Arena::new` and once at the snapshot's decode (the snapshot records the digest, so no format moves; `Arena: Clone` clones an `Arc`; the in-source terrain writers use `Arc::make_mut`), the only way a registered kernel reaches a 65,000-cell terrain from inside the engine without a copy per think or `unsafe`; the view is wrapped once after `mind_view`; the card crosses as `Arc<StockCard>` cloned once per think (24 integers). `ThinkResult` gains `fault: Option<ThinkFault { cause: SmolStr }>` with the seven words `ops`, `limit`, `type`, `memory`, `intents`, `unknown`, `panic` and `trace: Vec<String>`, which the arena drops on the floor (never hashed, never journalled; M6's trace ring); every `MindRunner` impl answers both. **D-UM-31 as built on the engine side**: `faulted = fault.is_some() || intents.len() > MIND_INTENTS_MAX` takes M1a.5's ONE atomic overflow path — nothing lands, the memory kept, `pending` cleared, the drained notices put back, the ranges restored, no `MindLand` — counts a never-hashed `ReflexCounters.mind_faults` (and `mind_overflows` on the length) and writes `strikes = min(strikes + 1, MIND_STRIKES_TO_DROP 3)`; a clean think resets to 0 only while `strikes < 3`, so **`strikes == 3` IS the drop** (`MindState::dropped()`), a rule on the existing hashed, carried field with no shape move; `ThinkJob.dropped` tells the runner, which answers with the native stand-in (M2.4; D-UM-32's ROM and the `MindDropped` line are M3's, as is the `MindFault` line §8 names). "The hash moves by strikes alone" is proven against the seventeen-intent overflow runner with `strikes` reset by hand — the one baseline on the same path (a never-faulting run takes the clean path and cannot hash equal); one event list and one hash sequence at 5,000/1,000/500 ms. **D-UM-29**: `DrawLabel::Mind { unit, think_seq, n }` is the sixth kind, encoded `mind:unit:seq:n`, `LABEL_KINDS` 5 → 6 (16b's `contact` was the fifth, so §7's "five-example array" reads six) with `one_of_each` and `arena_library`'s examples in one diff, drawn by nothing on a production path. Two byte-neutral kernel refactors for the port: `best_hostile`/`best_hostile_ranked` over `Weights { range, facet, health }` with `const fn Weights::of(&ReflexConfig)`, and `view_ranges(view, engage_band_permille, order)` over the band per mille, every caller updated (the twin's shadow among them). **Ruling (a)** on §5's target rule and the M1b.1 entry's "a `Lock`-target slot keeps firing at a lock whose row reads dead until the track expires, a ruling owed": `warrant` step 2 reads `!contact_reads_dead(held)` for `Lock` and `Contact { id }` as for `BestHostile` — one rule for the three targets — pinned by a cross-tempo fixture whose match outlives the kill (a second unseen blue hull; with the rule reverted the tape fires at 6,000 and 7,000, so M1b.1 did fire at the wreck). **Ruling (b)** on the M1b.5 entry's "`set` validates sliders independently … §11 asks for no cross-slider rule": `CardError::Pair { slider, fight_home, break_off }` refuses a `break_off_pct` or `fight_home_pct` that would leave `stand_min_permille > withdraw_permille` with `{slider} would put fight_home_pct {fight_home} above break_off_pct {break_off}; fight_home_pct is at most break_off_pct` (equal lawful, refuse-not-clamp, the card untouched on `Err`), and the new `StockCard::pair(named)` guards both preset paths (`apply_mind_card` and `@mind/set`), so a PRESET that breaks the pair on a config-admitted doctrine (`Rearguard`, `break_off_pct` 70, on a doctrine with `fight_home_pct` 75, found by the review) is refused by its own name and never clamped — a narrowing of D-UM-43's presets being always takeable, the default and shipped doctrines unaffected; `fight_home_pct`'s range sentence still says 0 to 100 though a value above `break_off_pct`'s 80 is always refused by the pair (a wording residual). **Ruling (c)** on the M1b.4 entry's "`range < | > n m` against the ROUNDED range; inclusive at the warrant while the reply says `under`/`past`, a ruling owed" and "the count is pre-checked off the mirror": `range < n` compiles to `max_range_m = n − 1` and `range > n` to `min_range_m = n + 1` — STRICTLY under and over at the rounded range, the warrant unchanged (`range < 1` stores 0) — and `parse_arming` answers `(FireCondition, ArmingEcho { min_permille, aspects, under_m, past_m, band, keeps })` so `arming_reply` echoes the TYPED figures and every M1b.4 reply literal is byte-identical, while `arming_words`, every renderer, golden and keyframe print the stored inclusive bound (`S1: >=10% under 599m`, the held line `outside 201 to 599`; the reply-versus-`WEAPONSTATUS` wording is the lead's); the cockpit's slot-count pre-check in `fire_when` is DELETED and the arena's phase-4 `NoSlot` is the one wall: an arming past a full table prints its acceptance sentence at once and the arena's `FIRE missiles refused. Every slot is armed; FIRE <weapon> OFF frees one.` arrives a pulse later as a notice (the two-line transcript pinned, with the one-pulse `FIRE mg OFF` then `FIRE missiles WHEN` race armed in the freed slot); two effects declared and pinned: `range < 200 AND band = close` (both 200 before) is refused `NotThisCore` as a differing duplicate (the rule stands, one figure per term), and `range > 200 AND band = effective`, refused under M1b.4, is now taken at (201, 400) — any range term one metre past a band edge flips the same way; `range > 200 AND range < 201` compiles to the empty window (201, 200), accepted, a residual to refuse. **Rulings (d) and (e)**: `slot_parallel` and the strict m1a6 assertion stand; the mind lane stays a by-hand run, no new manual job. The twin equal to M1b.5's record line for line, the six pins and the harness block equal: **no T0-versus-T1 divergence moved** in this sub-cut or any later one of M2.
- **§3 "The view" and D-UM-4, D-UM-6, D-UM-7 (the view types and parity), §4 "The mind's functions today" and D-UM-9, D-UM-10, D-UM-11, D-UM-17, D-UM-47 (the station functions), §7 "The budget", "Parameters", "Floats", "Randomness" and D-UM-26, D-UM-27, D-UM-28, D-UM-29 (the kernels, the fees, the facts, `rand`), §11 (the card type), §13 (`MIND_ABI`)**: as built (17b cut M2.3, 2026-10-05, `d45ee1b9`): `crates/tacsim/src/minds` (`mod`, `context`, `api`, `card`, `fees`, `kernels`, `stations`) under `#![deny(clippy::indexing_slicing)]` — §7's module-level add — with `API_VERSION` 1, `STOCK_VERSION` 1, `MIND_ABI = "rhai=1.26.0;api=1;stock=1"` composed from `RHAI_ABI` and asserted, `STOCK_OPS_PER_THINK` 1,500, `STOCK_LINE_TO_PER_THINK` 2 (D-UM-35's `line_to` column for class I, the cap's source), `RAND_MAX_K` 1,000, `register_api` through a `Registrar` that records every name it registers so `registered_names()` is the engine's list by construction (226 names pinned in registration order, the sandbox's and the packages' after; the five D-UM-11 verbs `order`, `release`, `place`, `board` and any exit asserted absent), `standard_sandbox()` one `OnceLock` per process at `EngineLimits { max_ops_backstop: STOCK_OPS_PER_THINK }`. **The per-think and doctrine facts are zero-argument native CONTEXT FUNCTIONS** — `lands_ms()`, `t_ms()`, `decide_ms()`, `cell_cm()`, `leash_cells()` 15, `station_cells()` 5, `screen_half_arc_mdeg()` 60,000, `stand_ticks()` 3, `exposure_front()`/`flank()`/`rear()` 1,000/600/200, `arrive_cells()` 1, `slots()` 2, `blocked()`, `cover()` — reading a thread-local `ThinkContext { view, map, seed, unit, think_seq, t_ms, lands_ms, decide_ms, draws, line_to_calls, intents, overflowed, refused }` the runner sets at the think's start and takes at its end (one think per thread at a time), fee 0, callable at any depth, never scope constants or script `const`s (the scope contract above). **D-UM-27 as built**: nothing is "in scope" — `fn think(view, card)` takes the view as its first argument and the card as its second, `this` the memory; one AST per `(digest, MIND_ABI)`. **The view (D-UM-7)** is eleven registered custom types, all `Clone + Send + Sync`: `View(Arc<MindView>)`, the `(Arc, index)` handles `Contact`, `Weapon`, `Section`, `Notice` and the `(Arc, section, facet)` `Facet`, `Order(Arc)`, and the copied `Heat`, `Receiver`, `Condition`, `Armed`; every getter answers an INT, bool, word, two-INT cell or `()` and never a FLOAT; **D-UM-6 as built**: a contact crosses as its `ContactId`'s 64 bits as an INT by `StockMemory`'s bit-cast (`View.lock`, `Contact.id`, `Notice.contact`, `Order.target`/`of`), the designator and alias as getters. **Parity with the keyframe (D-UM-4, §3's "every mind field is first a keyframe field") is at the `MindView` FIELD level**, not key-name identity, which the readers T1 needs cannot satisfy: a pinned 114-row `PARITY` table names each getter's rendering — the key of the same name, or the notice's `text` for a miss `reason`, the kind words `drive_arrived`/`drive_no_route` for a drive `outcome`, `expires_in_ms + t_ms` before the clamp for the absolute `Order.expires_ms`, the designator's row for `target`/`of`, `class != unknown` for `identified`, the array position for `Weapon.index`, the callsign's side for `View.team`, `held` for `blocked`; `Weapon.range_band` answers the four INTs `[index, of, max_range_cm, dispersion_mrad]` or `()` because the keyframe's object carries no word; the notice getters are one per `FrameNotice` field (`reason`, `outcome`, the `detail` keys `radius_m`, `secs`, `heat`, `max`, `floor_permille`, `why`, `floor`, `range_m`, `min_m`, `max_m`, `rounds`, `keeps`, `percent`, `aspect`), pinned at `FRAME_VERSION` 3; **a D-UM-4 reading**: `Receiver.reach_permille` is floored to the keyframe's whole per cent (`/ 10 * 10`) — a tier never extends, and the fold's raw per mille would hand a mind a digit no pilot sees. **The card (§11)** is `Card(Arc<StockCard>)`, read-only, its 24 STORED fields in stored integer units named by their slider where the units agree (`band_permille` for `band_pct`, `withdraw_permille` for `break_off_pct`, `stand_min_permille` for `fight_home_pct`, `rally_ms` for `rally_s`, …) and no derived figure: a half-percent slider cannot cross as an INT percent and D-UM-28 forbids a FLOAT; the eleven `[M2]` sliders are exposed and read by no stock script (M5/M6). **D-UM-17 as read, and §4's list as built** (17b §1's departure (2)): `fire(weapon, slot)` → `Fire { weapon, slot: Some(n) }` (the arena numbers slots; a mind always cites one), `arm(weapon, target, min_permille, aspects, keeps, min_range_m, max_range_m)` → `Condition(Arm(FireCondition))` with the seven terms of a condition and `target` `"lock"`, `"best"` or a contact INT, `disarm(weapon or ())` → `Condition(Disarm)` — not `fire(slot, condition)`, `arm(slot, condition)` and `disarm(slot)`, and no builder type, so the station list stays exactly §4's twelve (`helm`, `speed`, `startup`, `goto`, `hold`, `goto_off`; `lock`, `emcon`; `fire`, `arm`, `disarm`; `trace`); `arm` bounds `min_permille <= 1000` and `aspects` in `1..=15` as `type` (D-UM-10: a mind arms no slot a pilot's `FIRE … WHEN` cannot write; `min_range_m > max_range_m` is not refused because `parse_arming` accepts an empty window too), and an INT target is not checked against the view's rows (the landing's `Unlawful` is the one wall; a residual). **`goto` is registered as custom syntax `goto ( $expr$ )`**, not a function: rhai 1.26.0's tokenizer reserves the word and refuses it as a call; the script spelling is unchanged (§4's `goto(cell | spot)`), the cell or spot word evaluated as any expression (a spot through `cockpit::resolve_spot`, the one resolver). **The order of checks** (D-UM-9, D-UM-47, §22's italic line): every station argument is `Dynamic` and its TYPE is read first (a FLOAT or an out-of-range figure `type`, never a rhai lookup error a script could catch); then on the fire station THE UNGRANTED CHANNEL FIRST — a hull with no weapon rows has no `Fire` channel, and `fire`, `arm` and `disarm` there push their intent and write `fire: this hull has no weapons station (refused at landing)` to the trace with no refusal and no fault, the landing refusing the intent `Channel` through the one door into the mind's notice buffer (checked before the unknown-weapon check because on an unarmed hull every weapon id is unheld); then an unknown weapon, slot (outside `1..=slots()`), contact or spot `unknown`; then the push onto the sixteen-deep buffer in call order, a seventeenth refused `intents` with the context marked `overflowed`. **Every refusal sets the context's first word and is raised through `mush_mind::system_error`, uncatchable** (D-UM-31). **The kernels (D-UM-26)** are each a thin registered wrapper over the ONE free function T1 calls, never a second body, with the fees as `fees.rs` constants charged through `mush_mind::budget::charge` at the call: `FEE_THINK` 20; `best_hostile(v, w_range, w_facet, w_health, health, prefer, ceiling_cm, facing_mdeg, half_mdeg)` over `stock_mind::best_hostile` with T1's stand admit, 4 + 3 a row; `Contact.aspect`, `Contact.odds(weapon)`, `is_dry`, `would_overheat`, `fight_limit_cm` and `contact(view, id)` 1 — the row lookup, so the port's cost stays flat in the row count as §18 intends, answering `()` for an unheld id and never a refusal; `holds_a_hostile` 1 + rows; `health_permille` 2; `ranges(view, band, obey_order)` 2 over `view_ranges` with `obey_order` the SCRIPT's obeyed-order local, as T1 passes `obeyed(view, mem)` and not `view.order` (an ignored or ended order carrying `max_range_band` must not cap T2 where it does not cap T1; mirrored); `line_to(view, contact)` 20 over `line_between`, counted before the cap, a third call a `limit` refusal; `entry_cell`, `far_entry_cell`, `stand_off_cell`, `patrol_step`, `fall_back_to_waypoint` 30 over the motor's pickers through `ctx.map`; `rand(k)` 2 over `DrawLabel::Mind { unit, think_seq, n: draws }` with **`1 <= k <= RAND_MAX_K` 1,000, else `type` and no draw** (D-UM-29 as built: a raw draw is invertible back to the seed-keyed FNV state through a label the script knows, `rng.rs`' two bijections, while ten bits of each of many distinct draws are not; the one-way subkey the review proposed bends D-UM-29's letter and is the lead's); `sin_m`/`cos_m` (`SIN_ONE` = 1 << 20), `bearing_to`, `range_to`, `arc` and `stand_off_want` free; `hold_goal`/`hold_arrived` NOT registered — no production think carries a Hold objective (the view carries no match mode, `NativeStockMind::think` passes `hold: None`), so a kernel would have no caller and no Gate A oracle; M4 or a ruling adds them with the match mode on the view. Four `stock_mind.rs` functions (`line_between`, `own_point`, `stand_off_of`, `contact`) became `pub(crate)` for the wrappers, no body changed. **D-UM-28 as read**: "no float function is registered" means none registered by the game — the seven packages' float overloads are reachable in expressions, and a FLOAT reaching memory or an intent argument is a `type` fault; a wrong-typed REQUIRED kernel argument (`rand(1.5)`, a FLOAT weight) is rhai's own catchable not-found rather than a latched refusal (declared, harmless while kernels push nothing, not pinned). The package methods are pinned by package, not name by name (rhai 1.26 enumerates natives only under its `metadata` feature).
- **§6, §7 "The engine", §8 "Faults" and "Doctrine", D-UM-22, D-UM-31, D-UM-32 and §22 M2's italic line (`RhaiRunner`, the faults and the drop)**: as built (17b cut M2.4, 2026-10-05, `9d11780e`): `minds/runner.rs`' `RhaiRunner { sandbox, compiled, stand_in: NativeStockMind, decide_ms, stats }` with `RhaiRunner::new(cfg, source, digest)` on `standard_sandbox()` (a test constructor `on(sandbox, ..)`) is a `MindRunner` that runs the script SYNCHRONOUSLY at the think point exactly as `NativeStockMind` runs — the lead's note; D-UM-22's pool and the batch awaited before a snapshot are M4's — and sits on no production path: `default_mind_runner` is still `NativeStockMind`, through a snapshot decode included, pinned. A think sets the context from the job (both `Arc`s cloned, the seed, the clocks), runs `Think { args: [View, Card(Arc::new(card.clone()))], memory, limits: STOCK_MEMORY_LIMITS, ops_limit: STOCK_OPS_PER_THINK, fee_per_think: FEE_THINK }`, takes the context back and **labels the think on BOTH ends of the call** — the context's refusal word first (`intents`/`unknown`/`limit`/`type`), then its overflow flag, then the sandbox fault's own cause word — so a refusal a script caught, an overflow on a clean return or a trailing fee still faults; on any word `intents` is empty and `memory` is the job's bytes (the arena's one atomic path keeps the old memory); a dropped job (`strikes == 3`) is answered by the native stand-in with `DROPPED_LINE` as its whole trace — **D-UM-32's drop is native T1 at the card's own preset until M3's ROM and `MindDropped`**; `recording()` turns on `ThinkStat { unit, think_seq, ops, fee, rhai_us, faulted }` read off the budget after either end (the wall timer the only clock the runner reads, never touching the answer; a dropped think records none). `tests/minds_rhai.rs` proves the runner on the live (j) field through `Arena::mind_think`: every fault word with its fixture and its `try`/`catch` variants — `loop {}` `ops` at the same count on two runs, a 13-deep recursion and `s += s` `limit`, `helm(1.5)` and `this.x = 1.5` `type`, a 33rd key `memory`, seventeen `helm`s `intents`, `lock(999)` `unknown`, a third `line_to` `limit`, a panicking `boom()` on a second sandbox `panic`, `exit()` NOT a fault — each landing nothing, keeping the memory and counting a strike, three running dropping the hull with the stand-in's fourth think equal to `NativeStockMind::think` on the same job and the substep hashes equal to the seventeen-intent overflow baseline; every station function landing through the one door at the recorded epochs in call order with `trace` in the result and in no ledger line, the ungranted fire refused `Channel` at the landing with the `Refused` notice buffered and `strikes` 0 (§22's "a refusal in the trace, not a fault"); all 114 `PARITY` rows against `status_frame_held`, each on a value the keyframe printed, over six moments (the shapes the (j) field never produces — the heat cell, a pod switched off, a degraded receiver, the standing orders, twelve notice shapes — hand-staged on the mind row); every plot kernel holding the plot while the truth moves and equal to its free function, `ranges`' order arm both ways (`ranges(view, band, false)` under an ended order carrying `max_range_band` equal to `view_ranges(view, band, None)`); `rand` on the arena's seed under the sixth label (two runs equal, a reseeded arena different, the journalled draws equal to a no-draw run); a `@mind/set`-staged card read at the next think; and the MANDATORY cross-tempo fixture of a scripted Rhai mind (drives, holds after `drive_arrived`, locks, writes `lands_ms` into memory, traces) with one event list, one hash sequence and one trace sequence at 5,000/1,000/500 ms on both drives. Declared and not changed: `label()` ranks a latched refusal above `Fault::Panic`, the reverse of `mush-mind`'s own order, unreachable while no registered production function panics (M3's `MindFault` journal settles which word wins); the precedence of an earlier latched refusal over a later overflow is pinned in-source only.
- **§11 and D-UM-43, D-UM-44 (Standard Doctrine as content), §17 Gate A, §18, D-UM-32, D-UM-35, D-UM-48 (the port, the twin and Gate A)**: as built (17b cuts M2.5 and M2.6, 2026-10-05, `89274f1a` and `60440c44`): `crates/tacsim/content/minds/standard.rhai` (1,112 LF lines) with its manifest `standard.json` `{ name Standard Doctrine, role combat, api 1, stock 1, rated_class I, ops_ceiling 1500 }` (D-UM-48's role declared; `MindManifest` serde with `deny_unknown_fields`), `include_str!`ed in `minds/content.rs` with `STANDARD_SHA256` (the 64-character lowercase digest) pinned by `tests/content.rs` against the on-disk bytes, the manifest's six fields, the header line, a compile with `cached()` 1, the limits and no FLOAT, re-blessed ONCE (M2.6: the fire half written and four cost-only helm-half rewrites — `read_notices` as one `switch`, the leash radii only under a leash, `p.dry` and the range inline in the stands — that brought the 2v2 lanes from 1,207 to 1,226 ops down to 1,179 under the asserted 1,200), and `standard_runner(cfg) -> Result<Arc<dyn MindRunner>, CompileError>` compiling it on the process's sandbox (the source store `tacsim:mindsrc:<sha256>` of §13 is M3's; nothing of production names it). **`decide_on_card` is ported ARM FOR ARM under the binding limits** (the reviews' F9: a one-body port would pass the function expression depth 24, which binds every body with a level per block, `if`, `else if`, loop and `switch`): `switch view.power` and `switch this.state` dispatch with the state codes as integer literals (0 startup … 5 disabled) pinned against `ReflexState::ALL`, one helper per state arm (`patrol_arm`, `approach_arm`, `engage_arm`, `withdraw_arm`) and per T1 helper (`search_goal`, `hunt_goal`, `walk`, `stand`, `withdraw_stand`, `engage_orders`, `keep_station`, `look`, `arming_for`, `aspect_mask`), the edges merged into `unreachable`, `standing_order`, `edges`, `hostile_edges` and `leash_arms`, the working values on one map `p` handed to each helper and returned — 30 functions of the cap's 32, deepest body at expression depth 22 of 24, widest scope 15 of 64 (`arming_for`), measured by the pin and reported; `fn think(view, card)` with `this` the memory and the prologue `if !("state" in this) { this.fresh() }` writing all fourteen keys so the first think's bytes are T1's; every per-think and doctrine fact a context function; the row lookup `contact(view, id)` and the only loop over `view.notices`; `weights(card)` deriving `as_reflex`'s triple by `prefer`; `ranges(view, card.band_permille, p.order != ())` off the obeyed-order local after the early ends; `RANGE_NONE` (`i64::MAX`) for an unheld row; every site T1 saturates clamped in plain arithmetic (`min(range_m, 92233720368547758) * 100`, `min(t, MAX − dwell) + dwell`, the search, hunt and rally clocks as `since <op'> t_ms() − card_ms`), pinned at the extremes; `rally_ms` 0 never (T1's overlay writes `u32::MAX`, a rally at 49.7 days; unreachable by any match); the Hold square's three sites unreachable (no match mode on the view); `arming_for` arming each weapon of the first `slots()` in component order WHATEVER THE HEAT on the card's discipline (`arm(weapon, "best", card.min_odds_permille, aspect_mask(card.aspect), reserve_pct × rounds_max / 100 held to a u32, 0, min(ceiling, reach) / 100)`), only where the view's `armed` is `()` or differs in a term other than the slot number, `disarm(())` once beside an armed row when no ceiling admits, nothing on an unarmed hull (D-UM-48's absent fire half); `engage_orders` recording the admitting ceiling past the power, hold-fire and dry gates and citing the first armed weapon ready at the landing — **`ready_in_ms <= lands_ms() − view.t_ms`, the VIEW's clock as T1 reads it**, not the job's `t_ms` (the brief's `max(lands_ms() − t_ms(), 0)` would have diverged; the cited-shot mirror pins the boundary: ready exactly at the landing cited, one millisecond later not) — loaded, in reach and not overheating, numbered `armed.slot` else `i + 1`; the leash stand, the screen's stand and the withdraw stand calling it as T1 does (`release_cm` home, `stand_cm` en route); the emission `arming_for` first after `startup()`, then the helm half, `lock`, then `fire(p.fire[0], p.fire[1])`. The script does not normalise a foreign or ill-shaped memory as `StockMemory::from` does (the prologue runs `fresh()` only when `state` is absent), unreachable in M2 and a question for M3's mind switch. **Gate A (§17) as met, EXACT, and not as §17 wrote it**: there is no corpus file of harvested `(view, memory)` pairs; `TwinRunner` in `tests/support` runs `NativeStockMind` and `RhaiRunner` on the SAME `&ThinkJob` (both pure functions of the job), **asserts `fault.is_none()` FIRST** — printing the cause word, ops, fee, the trace and the view, because a Rhai fault on a quiet think would otherwise answer T1's empty intents and unchanged bytes and pass — then equal intents, then equal memory bytes, panicking on the first divergence with both answers printed, and flies T1's answer (so a `Twin` match hashes as T1), with `Count` mode for a report; every think of every mind hull is compared in place over THREE lanes in `tests/twin.rs`: (1) `m2_gate_a_over_the_pooled_lanes_matches`, the pooled lane's own thirty matches flown `(T0, Twin)` and `(Twin, T0)` per seed plus its control, 10,566 thinks (10,271 armed), with the lane's 67 printed lines equal to `M1B5_POOLED_RECORD` index by index — so `Twin` is T1 and the pooled record did not move; (2) `m2_gate_a_on_a_mixed_pool`, one armed and one unarmed hull a side over fifteen seeds at a 300 s cap, 12,764 thinks, where unarmed hulls are shot at, withdraw and are disabled (120 Disabled-edge thinks compared; D-UM-48's "the twin corpus covers such hulls" — an all-unarmed pool takes no damage); (3) `m2_gate_a_over_the_authored_lance_field`, twelve `Controller::Mind` a side mustered as the harness's lance lane musters (`support::lance_arena`: the fielded cohort, red on column 102 heading east, blue on 156 west, the rows' alternation, lances of four) under the lance lane's scripted order table re-expressed as `support::lance_orders` and its 90 s release, issued through the arena's own order path at each hull's epoch, three seeds, 120 arena seconds, 6,036 thinks compared (4,833 ordered, every `ordered` and `order_released` line reconciled against the living hulls, the contact-row cap reached on 3,635 thinks and the odds-row cap on 2,589), about 15 s debug so in the sweep; plus `m2_twin_against_twin_over_the_fifteen_seeds` unswapped (10,354 thinks), the unarmed fixture equal to the native run at 5,000/1,000/500 ms, `lance_field`, the two-T1 fixture (`M1B2_TWO_T1_FIRED` kept, the M1b.5 anchor equal), the (j) and charge fields and every `m1b2`/`m1b3`/`m1b5` T1 fixture of `tests/minds.rs` through `support::with_twin` (ten `_under_the_twin` siblings; `Stock` generalised over `Arc<dyn MindRunner>`), the laser, two-gun, hold-fire, retarget and stale-window fields of `tests/conditions.rs` through `at_three_cadences`' keep arm (145 thinks), and seventeen in-source mirrors through both runners (the eight no-fire unit fixtures of `stock_mind.rs`, the order cap off the obeyed order, the withdraw dwell and the rally, the Disabled edge from every key set, eleven cards' terms, the saturation extremes, and M2.6's four M1b.2 mirrors: `arming_for` on every preset at two ceilings, the admitting ceiling with `Lock`, `Contact` and `min_range_m` variants, the cited shot at the landing boundary, the removed-weapon residual through the arena). Lane 3's 120 s reach only the table's Engage and Hold arms with the release and the deadline (Advance, Withdraw, Screen, Support, the early ends, `ignore_until_ms` and the leash are held by the mirrors and the `minds.rs` siblings), and the rally and the dwell occur in no lane. **T0-versus-T1**: nothing moved — the twin's T1 record equal to M1b.5's line for line throughout M2, and T2 ≡ T1 exactly, so §17's Gate B figures are T2's without re-measurement. **D-UM-35 and §18 as measured** (ops plus fee): p50 755, p99 825, max 947 on the mixed 2v2 lane before the fire half; p50 992, p99 1,220, max 1,274 on the lance field after it (273 of 6,036 thinks past 1,200; `line_to` at most 1 of the cap's 2) — under class I's 1,500, over Class 0's 400 on almost every think (11,984 of 12,764 on the mixed lane), about THREE TIMES the ROM budget §18's `minds_stock_ci` gives Standard Doctrine at Class 0: the forward STOP for M3/M4 the lead rules on (plan Q2: whether a port above 400 may be Standard at Class 0, or M3 raises the ROM's class); the 2v2 lanes assert 1,200 as a STOP, lane 3 reports its figures (a bound there could never fail: a think past 1,500 is an `ops` fault the asserting twin panics on first).
- **§7 "Floats" and D-UM-28 (the cross-platform pin), §17 (whole-match hashes), §18 (the figures of record), §13 (the resume reinstalls the runner)**: as built (17b cut M2.7, 2026-10-06, `96887076`; test-side and CI only, `STANDARD_SHA256` unchanged): `tests/twin.rs::m2_whole_match_hashes_are_equal_between_the_runners_over_the_pool` plays each pooled match once T1-flown and once by `Flier::T2` ALONE (`standard_runner`, no twin) through `support::play` and holds the substep hash count and every hash (`Played.hashes` ACCUMULATED after every step, because `Arena::substep_hashes` is cleared at the head of every `step`), every ledger line and the ledger digest, the end, the stop and the final `state_hash` equal — 30 matches, 60 plays, 22,811 substeps, 18,897 lines, 0 divergences — `#[ignore]`d under 17b §1's thirty-second rule (30 s debug alone) and run by the named `test`-job step `Whole-match mind hashes (17b cut M2.7; Gate A)` on both hosts; every T1 fixture of `tests/minds.rs` and `tests/conditions.rs` re-flown on the Rhai runner ALONE at 5,000/1,000/500 ms through `support::rhai_alone`, `with_rhai` and `under_the_rhai_runner` and held to the native run's events, hashes and thinks (`M1B5_TWO_T1_EVENTS`/`INTENTS` now the Rhai runner's answers), with `m2_a_resumed_field_flies_the_rhai_runner_again_after_the_decode` encoding at a pulse boundary and **re-applying `with_rhai` after the decode, since a snapshot decode reinstalls `default_mind_runner`** (§13's seal recording the runner is M3's; the stitched hashes the native run's). **D-UM-28 as built**: `tests/minds_rhai.rs::m2_the_twin_corpus_hashes_the_same_on_every_host`, `#[ignore]`d and run by the named step `Cross-platform mind hashes (17b cut M2.7; D-UM-28)` before `Test` on `ubuntu-latest` and `windows-latest` — covered by the gated-test inventory's own scan, whose target rule became `invokes_target` so `--test minds` is not `--test minds_rhai` and a filtered `--ignored --exact` step covers only the test it names (no graft onto `vf_g6`, 10b's fixed list; the inventory keeps no census of ignored tests, its `*_gated` census of 13 unchanged) — flies seeds 2,350 to 2,352 T2 against T2 to the end AND `FLOAT_PROBE`, a scripted mind running twelve FLOAT expressions with `**` at non-integer exponents and mixed INT/FLOAT whose comparison outcomes fold into an INT bitmask emitted as `helm` and written to memory (the stock script is float-free by design, so the stock corpus alone could not catch the libm risk this test exists for), pinning per match the stop millisecond, the final `state_hash`, the SHA-256 of the substep hashes as little-endian u64s and the ledger digest (`M2_CROSS_PLATFORM_PINS`, `M2_FLOAT_PROBE_BITMASK` 369098751 on all 3,600 thinks) blessed on this Windows 11 host; **rhai 1.26.0 without `unchecked` compares FLOATs within `f64::EPSILON` relative**, so a one-ULP `powf` difference would read equal at a bare `==` — the probe's exact bits use `(v − exact) * 1000000.0 == 0.0` and six closing bits pin the tolerance itself: a comparison partly masks D-UM-28's risk, scaled arithmetic does not. Linux is unverified until the first push (the two steps sit before `Test` with no `continue-on-error`, so a red pin skips that host's workspace run; a parted probe is the lead's D-UM-28 ruling, never a quiet re-bless; the six pinned `powf` values are the correctly rounded ones). **§18's figures of record** (`twin_rhai_timing` over lanes 1 and 3 through `Timed`, release, alone; 17b §2 holds the row): ops per think p50 708, p99 1,123, max 1,182; fee p50 64, p99 98; ops plus fee p50 772, p99 1,208, max 1,274; ops per µs p50 11.9, p99 15.5, p1 5.2 — the script's own rate 11.9 ops/µs (9.7 in the parallel gate run), about HALF of §18's measured 19.5 to 24.1 on the blessing host itself, with `Timed` warming the cache on 1,320 of 16,602 thinks (report-only; it bears on the lead's Q2 and M4's pool sizing, and dev1's rate is still unmeasured); think_us Rhai 64/158 beside the native 4/9 and M1a.6's 3/10 with view_us 4/16; `view_wrap` p50 100 ns (the two `Arc`s and the card); `mem` p50 2,300 ns and p99 5,200 over 14 keys; compile cold 1,069 µs, hit 100 ns, `Sandbox::new` 163 µs against §7's 270 µs compile-plus-seal; the trace's extra ops 24; the script 30 functions, depth 22, scope 15, `line_to` max 1.
- **§22 M2 (the gate as met)**: sandbox tests — the identical budget trip on two runs (`mush-mind` test 1, at ops plus fee 501), no time, eval or keyword-function symbol resolving (test 3, with `sleep` shadowed in under 100 ms), memory canonicalisation (test 6 and `tests/minds.rs::m2_the_memory_codec_writes_the_native_bytes`), type faults (tests 3 and 4, and `minds_rhai.rs`' fault fixtures with their `try`/`catch` variants), the maximal Class III memory of 128 keys round-tripping with a 129th key `Memory(Count(129))` and a runaway `s += s` tripping the engine limit (test 8), the cross-platform hash test (M2.7: the stock corpus and the float probe, blessed on Windows, Linux pending the first push); Gate A, T2 ≡ T1 — on the pooled lane's thirty matches, the mixed lane and the authored twelve-a-side lane with every think asserted (M2.6), on the cross-tempo fixtures (M2.4's scripted Rhai mind, M2.5's unarmed fixture, M2.7's every T1 fixture under the Rhai runner alone at three cadences) and on whole-match per-substep hash sequences (M2.7); the italic line — the manifest declares `combat` (M2.5), the twin corpus carries unarmed hulls under fire and Gate A covers them (the mixed lane, M2.5 and M2.6), a call to an ungranted station function is a refusal in the trace, not a fault (M2.3's in-source test, M2.4's landing refused `Channel` with `strikes` 0); the lead's additions — `DrawLabel::Mind` (M2.2), the drop to the native stand-in (M2.2, M2.4), the three rulings with a test each (M2.2), the figures (M2.7), the production pins at every sub-cut and the harness at the sub-cuts that touched `arena/` (M2.2 and M2.7; M2.3 moved four functions' visibility there without the harness, declared); `Controller::Reflex` and the production `state_hash` unmoved throughout, no encoding re-bless; the cut's record is 17b §3, row M2.
