# Proposal — Unit minds: Rhai on the hull, the shot decision, the core, the lease and the Roll

**PROPOSAL, 2026-10-02.** Rests on the owner's seven pins of 2026-10-02 (P1 the shot decision and
no auto-fire; P2 minds are Rhai, sliders first; P3 one code path for AI fill, performance
permitting; P4 sealed at boarding; P5 a lease with a hardware requirement; P6 minds are tradeable;
P7 the REPL) and on the lead's architecture pins (the observation is exactly the cockpit's; the
outputs are the closed cockpit verbs through the same resolvers; think at N and land at N+1; an op
budget, never wall time; randomness from the arena's seeded draw; source hash, engine version and
parameter block in the `MatchRecord`; tiers are lore facts that bound engine cost). Parents: doc 16
D-16-2 (rate-independent tempo, due events), D-16-4 (integer state, keyed draws, a hash per
substep), D-16-6 (M32 is the unit truth), D-16-8 (per-mille resolution, the percentage is shown),
D-16-9 (closed consequence vocabulary), D-16-10 (contacts are an in-RAM shadow with their own id
space), D-16-16 (the reflex controller); W5, W7, W8, W12, W16; doc 11 §2 (the Warrant and the
Compact; habitat discipline); the Retreat proposal §8.4 and §8.5; 16c T-54 (per-channel leases
unbuilt). Synthesised from three angle designs (engine, player, economy), a read-only feasibility
pass over the code, and three judges. Nothing here changes shipped behaviour until §22's cuts land.
Decisions are D-UM-1 to D-UM-44 (§20); the owner's questions are Q-UM-1 to Q-UM-11 (§21).

Reading order for someone new: §1 (the shape), §5 (the fire law, which is where P1 lives), §6
(scheduling, which is where determinism lives), §9 (the tiers and the pool), §20 (the decisions).

---

## 1. The shape: three goods, one seal, one ladder

A unit mind is a sealed Rhai program that runs on a unit's compute core, reads exactly what that
unit's cockpit reads, and acts only through the cockpit's own verbs. Around the program sit three
goods and one record.

The **core** is hardware: a component on the hull with a Compact class. It sets what any mind on
that hull can do — how often it thinks, how many ops a think may spend, how many contact rows and
odds it is shown, how many condition slots and memory keys it holds (P5, §9).

The **licence** is law: a term lease on one filed mind, naming a holder, a seat count and a liable
warrant principal. A governed core refuses at the seal any mind the licence does not cover (P5, P6,
§10).

The **charter** is authorship: a titled object on the Registry's Roll that earns the licence revenue
and can itself be sold (P6, §16).

The **seal** binds hull, core, licence, revision and the parameter block into one immutable record
at boarding, written to the `MatchRecord` and the ledger's opening lines (P4, §19).

The owner and the players will say *mind*. The help says once what the fiction says: a unit mind is
not one of the Compact's frontier-class minds (the Retreat proposal §8.5: few, licensed like
reactors, located, audited; in play only as the deliberation lease W7 describes). It is habitat
discipline's automation (§8.4): it does, it does not decide; it is legible, overridable and
maintainable; it runs under a named principal's warrant. That is why sliders come first *in law* and
not only in the interface (§11): a filed mind must expose its discretion as a declared parameter
block, and the principal warrants the mind by setting those terms and arming its condition slots.
The Rhai body is the engineering; the card is the warrant's terms.

Everything a player meets is one **ladder** with five rungs, and every surface — the card editor,
the cockpit `MIND` block, the proving ground, the bench — serves every rung. Rung 1 picks a labelled
preset. Rung 2 moves sliders on that preset's card. Rung 3 writes condition rules
(`FIRE cannon WHEN odds >= 60 AND aspect = side,rear`). Rung 4 writes a Rhai `think` that calls
`stock::think` and overrides one part. Rung 5 writes a mind from scratch with its own card schema.
A new pilot never needs a rung above the one they are on; an author never leaves the ladder to reach
the top.

The engine spine is the engine angle's: a mind is a pure function
`mind(view, memory, card, keyed draws) -> (at most 16 intents, memory')`, run by a `MindRunner`
trait the arena calls, on a sandboxed op-only Rhai engine that lives outside
`crates/tacsim/src/arena`. The player angle supplies the surfaces (the ladder, the rule language,
attribution, the proving ground, the bench). The economy angle supplies the law (the warrant, the
governed core, the Roll, charters, the evidence-only crime loop). Where the three contradicted each
other the judges' resolutions are taken and recorded in §20.

---

## 2. What already stands, and what the designs got wrong about it

The feasibility pass read the engine. These facts bound the design, and each design asserted at least
one of them falsely; the corrections are folded in below and named here so the doc writer does not
re-import them.

| Standing item | Where | What it gives this document |
|---|---|---|
| The reflex is a staggered due event, `DueKind::ReflexTick`, every `reflex.decide_ms` (1,000) of arena time, resolved at its own millisecond; a per-pulse controller would break D-16-2 | `arena/reflex.rs` head; `step.rs:1508` | The scheduling pattern a mind must copy (§6) |
| `Arena::order` turns a plan into intents stamped with the current epochs, through `apply_intent` with `IntentSource::Machine`, resolved at `t_ms` by `resolve_startup`, `resolve_lock`, `resolve_fire`; its Emcon arm is a no-op | `reflex.rs:4567-4640`; `intents.rs:603` | The door minds land through, factored out (§4) |
| Every random value is `draw(seed, &DrawLabel)`; there is no stream and no generator state; `LABEL_KINDS` is pinned at five by the contract test | `rng.rs:1-22, :186`; `arena_library.rs:92` | Mind randomness is a sixth label, not a stream (§7) |
| The arena library gate bans eight substrings (`mush_store`, `tokio`, `HashMap`, …) in `arena/`; `rhai` is not among them but the interpreter still must not enter the library | `arena_library.rs:41-80` | The `MindRunner` seam (§7) |
| Substeps are cut from each pulse's own start; a late production wake ends a pulse off the lattice; pilot notices and held-loss settles are staged after the substep hash; `pending_notices` is cleared every pulse and must be empty in a snapshot | `clock.rs:575`; `step.rs:1586-1660, :1898, :3099`; `snapshot.rs:374` | Why "collect at substep end before the hash" is wrong and why minds carry their own notice buffer (§6) |
| Epochs are keyed `(unit, Channel)`; only `apply_control` raises one, then calls `set_controller`; `set_controller(Pilot)` calls `reflex.release`, which deletes the row; Linkdead hands nothing to the machine, only `Released` does | `state.rs:3042`; `intents.rs:470-555`; `reflex.rs:845, :1464` | Possess, release and the dormant mind (§12) |
| A three-channel cockpit seat holds no K6 lease; per-channel leases are unbuilt (T-54); the seat rate is one counter per unit, for `Seat` only | 16c T-54; `intents.rs:713-727` | Pilot-plus-mind channel splits are deferred (§12) |
| `Consequence` is closed: `DestroyUnit`, `RemoveWeapons{section}` (weapons only), `Slow`, `NoTurnToward`, `DisableMotion`; only `wreck_ops` revokes capabilities, and only power, mobility and weapons; `UnitMutationOp` has no install op | `definitions/unit.rs:275-335`; `damage.rs:940-970`; `mush-tactics unit.rs:1169-1240` | Core damage needs two new consequences; fitting needs an op (§9) |
| `Frame` carries contacts, lock, weapons, sections, heat, receiver, ECM, notices and catch-up; it has no standing-order field and no arena-time field; `FrameWeapon.p_permille` is against the lock only; `resolve_seen` reads the target's silhouette off the live unit | `step.rs:85-196`; `weapons.rs:550`; `resolution.rs:512` | The keyframe parity rule and the classification gate on odds (§3) |
| The reflex reads past the observer rule in four places: the `ObjId` tie-break in `best_hostile_where`, `reflex_has_line` tracing to the true pose, `ordered_target`'s Support branch reading the charge's own row or lock, and `contact_reads_dead` reading `target.fatal_sections` | `reflex.rs:3118-3290`; `damage.rs:493`; `perception.rs:1806` | The named divergences of the port (§17) |
| `mush-script` builds `Engine::new()` (the full standard package: `timestamp()`, float math, print to stdout) and its `on_progress` samples `Instant::now()` every 1,024 ops; `strikes_to_halt` is 3 | `mush-script/src/runtime.rs:567-610, :584-597` | Not reusable for minds; the three-strike rule is kept (§7, §8) |
| rhai is `{ version = "1", features = ["sync","serde"] }` at the workspace, resolved 1.26.0; every knob named in §7 exists in 1.26.0; `rhai::Map` is a `BTreeMap`; float `+ - * /` are built-ins with no package | `Cargo.toml:81`; rhai 1.26.0 `api/limits.rs`, `api/options.rs`, `func/builtin.rs:155` | The sandbox (§7) |
| The harness checks out **all 300 units** as `Controller::Reflex`; the 24 "pilots" are frame sinks; the lens is 12 ms per wake on both cohorts and 12 (CI) or 20 (authored) ms per 500 ms window; the baseline reads wake p99 6.55 ms, window p99 10.56 ms, one substep per wake, four wakes per arena second | `harness.rs:1897`; `harness_baseline/arena.windows.json` | The budget arithmetic is for 300 machines, not 276 (§9, §18) |
| A micro-benchmark on the blessing host (rhai 1.26.0, release): 19.5–24.1 ops/µs on mind-shaped code; 15.05 µs per think with custom types against 24.69 µs with maps at 12 contacts; 3.67–3.93 µs flat from 4 to 48 contacts when scoring is a native kernel; compile plus seal 270 µs; a zero-contact think 0.99–1.34 µs | `scratchpad/minds/rhaibench/bench-output.txt` | Every op number in §9 (§18) |
| `Representation::Entitlement` ("licences, programs"); `Legality` is recorded, rendered and filtered on, never enforced; `CatalogSource`, `AvailabilityProvider`, `DeliveryProvider` are wiring-frozen; `InstrumentKind::Entitlement` is *documented* non-transferable and unenforced; `issue_title`, `PayoutShare`, `DefinitionDigest::for_canonical_value`, `extraction_authorized`, `Lot.lineage`, `Delegation.expires_at` exist | `mush-market record.rs:125-215, module.rs`; `mush-economy record.rs:124, api.rs:2498, contract.rs:126`; `mush-core mechanics.rs:257`; `mush-industry licence.rs:156`; `mush-inventory record.rs:227`; `mush-org record.rs:212` | The market and the law reuse shipped seams (§10, §16) |

---

## 3. The observation: `MindView`, a projection of the cockpit frame

**The parity rule (D-UM-4).** A mind reads exactly what a pilot's `arena.cockpit` keyframe carries.
A tier may truncate the view; nothing may extend it. Every field a mind reads is first a field of
`step::Frame` and of the keyframe, with a `FRAME_VERSION` bump, so the observer rule holds by
construction: the view is built only from the per-platform builders a pilot's frame is rendered
from — `frame_contacts` (`step.rs:2883`), `frame_scope` (`:2909`), `frame_sections` (`:2977`),
`Arena::frame_weapons` (`weapons.rs:936`) and `lock_aim` (`resolution.rs:814`). A new
`Arena::mind_view(unit, &mut MindState) -> MindView` calls those builders directly. It cannot reuse
`Arena::frames` (`step.rs:1821`), which is `&mut self`, pilot-only, once per pulse, and drains
`pending_notices`.

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
what "the engine supports the shot decision" means in P1:

- *aspect*: the word `facet_band_seen` (`resolution.rs:306`) gives off the contact's own `SeenPose`
  (front, left, right, rear), advice from the plot as the reflex's exposure term already is;
- *odds*: one per-mille per weapon against any **classified** contact, computed by
  `Arena::resolve_seen` (`resolution.rs:512`) against a new `Arena::aim_at(platform, contact) ->
  Option<LockAim>`, which is `lock_aim` generalised to any held contact over `fire_cover_at`
  (`damage.rs:520`, a trace to the plot cell). Never for an unclassified contact: `resolve_seen`
  reads the silhouette off the live unit (`at.profile.band`), so an unclassified contact's odds would
  reveal its class. Only for the first `odds_rows` contacts the core's fire control resolves (§9);
  `CONTACTS` shows a dash where the core shows nothing;
- *reads_dead*: **not** the reflex's `contact_reads_dead` (`perception.rs:1806`), which reads
  `target.fatal_sections`, a fact about the machine. The view derives it from what the platform
  earned: its own gunner's kill notice, or a held `FrameCondition` reading that shows a fatal section
  destroyed. That is 16b cut E.2's rule applied honestly, and it is a named divergence of the port
  (§17).

On the 120-column `CONTACTS` row the two new columns read
`BA  blue  bearing 087° (rel 032°)  far  450 m  2 s  side  cannon 62%`. The bot's `Picture` parser
and the mush-ansi goldens change in the same cut.

**Two more fields join the keyframe** because minds need them and pilots may as well have them:
`order` (the standing squad order, with the commander's target hint already re-keyed to the unit's
own `ContactId`, §4) and `t_ms` (arena milliseconds, the only clock a mind has). The player angle's
"course-and-speed estimate" for C3 is **not** added: it is a derived input no pilot sees, so a tier
would be extending the view, which D-UM-4 forbids. A mind that wants a track keeps its own
`last_plot` in memory, as the stock mind does.

**Notices.** `pending_notices` is cleared every pulse and must be empty between pulses, so a think
that spans pulses cannot read it. `MindState.notices` is a bounded buffer (16 entries) filled by the
same post-hash journal scan that stages pilot notices (`stage_mind_notice` beside
`stage_pilot_notice`, `step.rs:1586`), hashed and carried by the snapshot, drained at each think.
Refusals of the mind's own landed intents are appended to it, so a refusal is never silent (D-UM-8).

**Contact identity (D-UM-6).** The API names contacts by the opaque `ContactId` the arena holds
(D-16-10's own id space), with the designator and alias carried as fields. An integer id costs no
string ops, survives a callsign rename or alias, and cannot be forged. The trace and the debugger
print designators.

**The Rhai types** are registered custom types with getters, never Rhai maps: the bench read
15.05 µs against 24.69 µs at 12 contacts (D-UM-7). `MindView` holds the unit's own facts (callsign,
team, cell, pose, speed, power, held, wreck, immobile, sight, ECM, receiver, heat, sections with
facets, weapons with slot index, ready, `ready_in_ms`, rounds, bands, heat per shot, range band and
`p_permille` against the lock), the lock and its odds, the standing order, `t_ms`, the drained
notices, and `contacts` truncated to the core's row count, nearest-first by held range with
`ContactId` as the tie-break. Each contact carries id, designator, class word, friendly, bearing and
relative bearing, range and band as `ContactRow` rounds them, age, cell, the earned condition bands,
aspect, odds per weapon (within `odds_rows`) and `reads_dead`.

**The map is public.** Map knowledge reaches a mind only through native kernels over
`ArenaDefinition` terrain, zones, waypoints and key spots, which a pilot already receives whole on
the `arena.map` feed (`cockpit.rs map_frame`). No kernel takes or returns an `ObjId`.

---

## 4. The outputs: the closed verbs, the motor, and a verb for pilots too

A think emits intents by calling registered functions that append to a native per-think buffer of
at most 16 entries; a 17th is a fault. The buffer is the think's return. Nothing touches the arena
until landing, and a faulted think lands nothing, so a think is atomic (D-UM-9).

**The doctrine that a machine can do nothing a pilot cannot is kept in both directions.** Each of
the three designs quietly widened the cockpit's verb set (a `Goal` intent; `STAND`, `CLOSE`, `OPEN`,
`TURN INTO` rule actions; a Command class that orders). The rule here (D-UM-10): a mind has no verb a
pilot cannot type, and every intent kind this proposal adds gets its cockpit verb in the same cut.
`COCKPIT_VERBS` grows from 14 to 16: `GOTO` and `MIND` (its length tests at `cockpit.rs:7934` and
`:9276` are reviewed changes).

The mind's verbs and their intents:

- `helm(mdeg)`, `speed(cms)`, `startup()`, `lock(contact)`, `emcon(on)` → `Heading`, `Speed`,
  `Startup`, `Lock`, `Emcon`, unchanged;
- `fire(slot, condition)` → `Fire` carrying a condition citation (§5); a mind never fires without
  one;
- `arm(slot, condition)` and `disarm(slot)` → a new continuous `IntentKind::Condition` on
  `Channel::Fire`, the same intent the pilot's `FIRE <weapon> WHEN …` stages (§5);
- `goto(cell | spot)`, `hold()`, `goto_off()` → a new continuous `IntentKind::Goal(MotorGoal)` on
  `Channel::Helm`, the same intent the pilot's `GOTO <spot|cell>`, `GOTO HOLD` and `GOTO OFF` stage;
- `trace(text)` → the trace ring only (§14).

A mind never issues `ORDER`, `RELEASE`, `PLACE`, `BOARD` or `EXIT`. Under the Warrant no automated
system may command (doc 11 §2), so `ORDER` from `IntentSource::Mind` is refused with `Channel`, the
word the resolver already has for a verb the source may not use (D-UM-11). The economy angle's
Command class is not adopted; whether it ever is, is Q-UM-5. The LLM deliberation layer keeps
issuing `IntentKind::Order` exactly as today, because that is W7's leased advisory mind speaking
through the squad's human-warranted doctrine, which doc 16 §6 already carries.

**The motor (D-UM-3).** `reflex.rs` does two jobs. Deciding — the state machine, the utility score,
goal precedence, the withdraw and rally clocks, fire politeness — moves into the Rhai stock mind.
Driving — `follow` (`:3616`), `steer_aim`, `first_step_is_open`, `line_is_open`, `walk_is_open`,
`braking_speed_cms`, the avoid list, the path budget (`ReflexMemory::may_search`) and the
per-substep `hold_routeless` pre-pass (`:1247`) — stays native as `arena/motor.rs`, owned per unit
as a `MotorRow`, driven by `DueKind::MotorTick` (today's `ReflexTick` renamed and stripped of
`decide`), with every bound unchanged (`path_max_cells` 65,536, `repath_ms` 4,000,
`max_path_queries` 64 per window, `STEER_LOOKAHEAD_CELLS` 8, the wander rule). Path search in an
interpreter would cost about a thousand times its native price; this is why the motor does not move.
`ReflexCounters` becomes `MotorCounters`, which moves the harness baseline keys in the same cut.

The motor is the hull's drive-by-wire and the pilot's `GOTO` drives it too: a pilot who types
`GOTO ridge` gets the same bounded A\* the machine gets, and `HEADING` or `SPEED` typed afterward
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

**Squad orders reach minds (D-UM-15).** `apply_intent`'s two `controller != Reflex` gates
(`intents.rs:633, :676`) become "is a machine", the order is written to `MindState.order` instead of
`ReflexMemory::set_order`, and expiry stays arena-side on `StandingOrder::expired_at`. The
commander's target hint is **not** resolved as `ordered_target` does today (`reflex.rs:3118`
resolves it in the issuer's contact space to an `ObjId`): it is re-keyed at write time through
`draw_contact_id(seed, unit, target)` and kept only if the unit itself holds that contact, else
none. `Objective::Support{of: ObjId}` is re-keyed the same way to the charge's `ContactId` or none,
and the Support branch's read of the charge's own reflex row or lock is dropped: a supporting mind
follows what it can see. The `Refusal::NotReflex` wire string `not_reflex` is kept until
`Controller::Reflex` is deleted (§17), when it becomes `not_machine` in one reviewed cut.

---

## 5. The shot decision and the fire law (P1)

P1 is the centre. Real weapons will have multi-second cycles and precious ammunition; there is no
auto-fire button; what the engine supports is the *decision* — odds for any contact, the target's
aspect, ammunition salience — and the era's SOP is that the human drives the conditions and the
machine implements. Three designs put this in three places (native slots; rules evaluated at think
cadence; a resolver refusal). The synthesis is one mechanism with two surfaces and one law.

**The mechanism (D-UM-16).** A **condition slot** is native engine state on the unit:
`FireCondition { weapon: slot, target: Lock | Contact(ContactId) | BestHostile, min_permille: u16,
aspects: mask over front/left/right/rear, min_rounds_left: u32, max_range_m: u32 }`, up to the
core's `condition_slots`, held in `MindState.conditions` (and on a piloted unit, in the same row),
folded into the hash. A native pass `Arena::evaluate_conditions(unit, t_ms)` runs at the weapon's
own `DueKind::WeaponReady` (whose arm is a no-op today, `step.rs:1483`), at every sensor pass that
refreshes the conditioned contact's plot, and at every landing. It reads the same per-contact odds
and aspect the view carries — the plot through `aim_at` and `resolve_seen`, never the target's live
state. When a condition holds it builds `IntentKind::Fire`, resolves it at that millisecond through
`apply_intent` and `resolve_fire`, which charge the cycle and the ammunition as any shot does, and
the fired ledger line gains `by: cond` plus the slot. Evaluation is bounded by slots × events and
costs no mind ops: the hardware paid for the slot.

This is also the fix for the bot's cadence finding. A typed `FIRE` pays the pulse between order and
arena (accepted no sooner than about 1.25–1.5 s after a 1 s cycle) while the machine fired on its
own tick; now an armed slot fires for pilot and mind alike on the `WeaponReady` millisecond.

**The law (D-UM-17).** Every mind-issued `FIRE` cites a slot: the Rhai `fire(slot, condition)`
is a one-shot citation evaluated at landing like any slot. A mind that emits a `FIRE` whose cited
condition does not hold on the plot is refused with a new `Refusal` word, `Unwarranted`
(`intents.rs:327`, `as_str` at `:402`; it reaches the wire and the cockpit sentence table). A
pilot's typed `FIRE` is the human trigger W8 asks for and is never gated. The **legal floor** is
validated at the seal, not in the resolver, so it cannot touch a running match: no lawful slot names
an unclassified contact (`BestHostile` ranges only over classified contacts; `Contact(id)` must be
classified when evaluated, else the slot is idle), and every lawful slot carries at least the Compact
odds floor, `[tacsim.minds].odds_floor_permille`, seed value 100. A card whose slot sits under the
floor is refused at the seal with the floor named: refuse-not-clamp. The player angle's `weapons:
free` mode does not survive as a mode: "free" is the labelled SOP `BestHostile` at the floor, which
the Line preset arms, and it is printed as a condition like any other.

**The surface (D-UM-18)** is the player angle's rule language, which compiles to slots:

`<ACTION> [<weapon>] WHEN <condition> [AND <condition>]…`, at most four conditions, no `OR` (two
rules are clearer than one), with the English explanation written back on save
(`R1: fire the cannon at your lock when your chance to hit is 60% or better and it shows you a side
or its rear.`). Conditions: `odds [weapon] >= n`, `aspect = front|side|rear` (a comma list,
identified contacts only), `range < | > n m`, `band = close|effective|far`, `target =
identified|moving|stopped|hurt`, `target <section> = green|yellow|red|destroyed` (an earned
reading), `rounds [weapon] < | > n | n%`, `reserve`, `own <section> < n%`, `hit within n s [from
front|side|rear]`, `hostiles >= n`, `heat > n%`, `order = …`, `lock`, `no lock`. Actions: `FIRE
<weapon>`, `HOLD FIRE <weapon|ALL>`, `LOCK nearest|best_odds|weakest|flank|newest` (for a `LOCK`
rule the target conditions are tested on each candidate). The helm actions the player angle listed
(`STAND`, `CLOSE`, `OPEN`, `TURN INTO`, `HUNT`, `BREAK OFF`) are **not** rule actions: they are the
stock mind's posture sliders (§11) implemented through `goto` and `helm`, because a rule action must
be a cockpit verb and those are not. A rule whose conditions fit a slot compiles to one; a rule that
needs a think (`hit within`, `hostiles >=`, `order =`) is evaluated by the stock mind at its think
and arms or disarms slots. The card says which is which (`R1 … (armed on the core)`,
`R2 … (checked each think)`).

Precedence fits in four lines and puts safety first: an `ORDER`'s `hold_fire` beats everything, and
it is now enforced for `IntentSource::Mind` and for condition fires in `resolve_fire` (today it is
honoured only in reflex planning, `reflex.rs:2862`; new resolver code, D-UM-19); a matching `HOLD
FIRE` beats any `FIRE` on the same weapon; then the first matching rule wins per channel; rules beat
sliders.

Pilots get the same slots from the same core: `FIRE cannon WHEN odds >= 60 AND aspect = side,rear`
typed in the cockpit stages `IntentKind::Condition`, `WEAPONSTATUS` shows the arming
(`cannon  ready  14 of 40  62%  effective  [armed: ≥60% side/rear, keeps 8]`), a typed `FIRE` is
arbitrated by the weapon's cycle against a slot's fire (whichever lands first fires; the other reads
`Cycling`), and every slot shot is announced once (`Cannon fires at BA by R1 (62%, side).`), with a
held shot said at most once per contact per 10 s (`R1 held the cannon: 54% under your 60%.`).

---

## 6. Scheduling: a due event at the lattice millisecond, landing one substep later

The lead's pin is to think on pulse N and land at N+1. A per-pulse controller breaks D-16-2, and the
engine angle's "collect at substep N's end, before the hash" is wrong twice: substeps are cut from
each pulse's own start so a late wake ends off the lattice, and notices are staged after the hash.
The reflex's pattern is right, and this proposal copies it (D-UM-20).

**The think is a due event.** `DueKind::MindThink { unit, think_seq }` is scheduled every
`think_ms` of the core's class, at the lattice millisecond `stagger_ms(seed, unit, think_ms)`
(`rng.rs:251`, the sensor pass's and the reflex's own rule) plus `k × think_ms`. It resolves inside
whichever substep its timestamp falls in, ordered among that substep's due items by timestamp, then
kind, then unit id, after the sensor passes, impacts and `WeaponReady` of the same millisecond. At
that instant the arena builds the `ThinkJob { unit, think_seq, t_ms, view, memory bytes, seal,
epochs }` from the state as it then stands; the view's notices are the buffer as filled by every
post-hash scan up to the end of the previous substep. That is the snapshot of N.

**The landing is a due event.** `DueKind::MindLand { unit, think_seq }` is scheduled at `t_ms +
substep_ms`, the match's substep length read when the think was collected (250 ms on shipped
content; `Clock::substep_ms` is a function of config, cell size and the field's top speed and not of
the tempo, which is why the cross-tempo fixture holds). It resolves in whichever substep its
timestamp falls in, before that substep's motion if it falls on the substep's first millisecond, in
unit-id order, each unit's intents in emission order, through `apply_machine_intents`. That is N+1:
one lattice step of arena time, never a tempo pulse, so the event order is the same function of
arena time at every tempo. The cross-tempo fixture extended with two mind units is a gate (D-UM-21).

**Execution policy (D-UM-22).** Jobs are pure functions of frozen inputs, so where and when they run
cannot change a result; only wall time moves. The arena task (`arena_task.rs`, outside the library,
tokio allowed) owns an `Arc<dyn MindRunner>` into a process-wide pool of `[tacsim.minds].workers`
threads, default 2, shared by every arena task and served first-in first-out per arena batch. When a
wake is one substep (the Engaged baseline: one substep per wake, four wakes per arena second) the
jobs collected in that step are sent to the pool after egress and awaited on a oneshot before the
next step: think time is off the wake's critical path and outside the wake lens. When a landing's
substep falls inside the same wake as its think (a Quiet pulse, catch-up, the harness's jittered
wakes) `Arena::step` calls the runner synchronously at the landing point, inside `catch_unwind` and
the stall watchdog, and the lane counts that time in the window. Backpressure: a batch is bounded by
the arena's pool ceiling (§9), so the wait is bounded; a batch that is still running when its arena
is due is awaited, never skipped, and the lane reports `pool_wait_us` so a saturated pool is seen as
late wakes, not as a different result.

**Snapshot and resume.** A snapshot taken between a think and its landing carries no job. The
`MindState` row carries `pending_think_seq` and `pending_t_ms`; resume re-runs `mind_view` at that
timestamp against the restored state and recomputes the job, which gives the same result because
the inputs are the same bytes. A fixture takes a snapshot between N and N+1 and asserts the hash
sequence (D-UM-23).

---

## 7. The sandbox and the budget

**The crates (D-UM-1).** `crates/tacsim/src/arena/mind.rs` holds `MindState`, the two due kinds,
job collection, landing, hashing and snapshot rows, and `trait MindRunner { fn think(&self, job:
&ThinkJob) -> ThinkResult; }`. It names no Rhai type, so `arena_library.rs` keeps passing and the
arena is tested with a native runner. `crates/mush-mind` is a game-neutral Rhai sandbox for unit
minds depending on `rhai` alone: the engine builder, op-and-fee accounting, canonical memory
encoding and hashing, the seal cache. `crates/tacsim/src/minds` holds the TacSim API on that engine
(view types, kernels, intent functions), the stock mind as content (`content/minds/standard.rhai`
plus manifest) and the `RhaiRunner`. The registry (§10, §16) is a separate crate, `mush-roll`, with
economy and org edges and **no** `mush-script` edge: conformance compiles through `mush-mind`, never
through the world engine. `mush-script`'s engine is deliberately not reused: its `on_progress`
samples a wall clock and its packages carry `timestamp()`.

**The engine (D-UM-24).** One shared `Engine` per process, rhai pinned **exactly** `=1.26.0` at the
workspace (which pins `mush-script` too, harmlessly), features `sync` and `serde`, so `Engine` and
`AST` are `Send + Sync` and the pool shares one engine and one AST per seal. Built with
`Engine::new_raw()` plus `LanguageCorePackage`, `ArithmeticPackage`, `LogicPackage`,
`BasicIteratorPackage`, `BasicArrayPackage`, `BasicMapPackage`, `BasicStringPackage`. Excluded:
`BasicTimePackage` (a wall clock), `BasicMathPackage` (transcendentals and parsing),
`MoreStringPackage`, `BasicFnPackage`. `DummyModuleResolver`; `eval`, `import`, `export`, `Fn`,
`call` and `curry` disabled through `disable_symbol` (the mechanism `mush-script` uses for `eval`;
that it disables the keyword-functions too is asserted by a test). `on_print` and `on_debug` are set
(`new_raw` installs none) and write to the think's trace buffer: at most 8 lines of 120 characters,
never journalled.

Limits (D-UM-25): `set_max_call_levels(12)`, `set_max_expr_depths(48, 24)`,
`set_max_string_size(128)`, `set_max_array_size(64)`, `set_max_map_size(64)`,
`set_max_variables(64)`, `set_max_functions(32)`, `set_max_modules(0)`,
`set_strict_variables(true)` (a typo is a seal-time error), `set_fail_on_invalid_map_property(true)`,
`set_allow_anonymous_fn(false)`; loops allowed because ops bound them; `set_max_operations` at the
tier ceiling as a backstop only. Native integer helpers: `abs`, `min`, `max`, `clamp`, `isqrt`,
`sin_m` and `cos_m` over `motion::sin_fx`'s table, `bearing_to(cell)`, `range_to(cell)`,
`arc(a, b)`.

**The budget (D-UM-26).** Thread-locals hold the think's op limit and a fee counter.
`on_progress(ops)` aborts with the cause word `ops` when `ops + FEE > limit` and never reads a
clock. Kernels add their fee when called and the trip lands at the next op. Rhai's operation count
is a deterministic function of the AST and the engine version, so the trip lands at the same point on
every host. Fees are calibrated once on the blessing host at about 42 ns per op (1/24 ops per µs):
`best_hostile` 4 + 3 per row; an odds or aspect read 1; `line_to(contact)` 20 and capped per think by
tier because it is a trace; each nav kernel 30; each `rand` 2; a fixed 20 per think for scope setup
(measured 0.99–1.34 µs at zero contacts).

**Parameters (D-UM-27).** The card is injected as a read-only custom type `card` in scope, not folded
into the AST: one AST per `(source_sha256, MIND_ABI)`, so 300 machines on 300 differently tuned cards
compile once (the engine angle's `optimize_ast` folding would compile once per card). Compile plus
seal measured 270 µs, paid at the seal in the world, never in the arena.

**Floats (D-UM-28).** Rhai's float operators are built-ins and the workspace cannot apply `no_float`
to minds alone (features unify). Floats may appear in expressions; no float function is registered;
memory values and intent arguments are INT only and a FLOAT reaching either is a type fault (D-16-4
carried into the script). Cross-platform determinism of basic IEEE operations between the Windows
blessing host and the Linux production build is asserted by a CI test over the twin corpus
(Q-UM-9 holds the alternative).

**Randomness (D-UM-29).** A sixth `DrawLabel`, `Mind { unit, think_seq, n }`, `n` the call index
within the think; `rand(k)` returns `draw(seed, label) % k`. A label, not a stream: a mind's draws
never shift another draw. `DrawLabel::LABEL_KINDS` and the free `LABEL_KINDS` (`rng.rs:74, :174`)
and the five-example array in `arena_library.rs:92` change in one reviewed diff.

**Lints.** `mush-mind` carries `mush-script`'s deny list (`unwrap_used`, `indexing_slicing`, `panic`,
`unreachable`, `panic_in_result_fn`). `crates/tacsim/src/minds` sits under tacsim's crate deny,
which omits `indexing_slicing`, so that module adds it at module level. `catch_unwind` across a Rhai
callback is the last backstop, with the stall watchdog.

---

## 8. Memory, faults and the drop to doctrine

**Memory (D-UM-30)** is `mem`, a Rhai map in scope that persists between thinks. Keys at most 24
characters, the key count capped per tier; values INT, bool, unit, a string of at most 32 characters,
or an array of at most 32 INTs, checked when the think ends. Because `rhai::Map` is a `BTreeMap` it
encodes canonically; `state_hash` folds its FNV digest and the snapshot carries the bytes. The arena
holds bytes, never a `rhai::Map`, so Rhai stays out of `arena/`. Memory starts empty at check-out and
is the reflex's decision fields made generic: `state`, `target`, `last_plot`, `unheld_since_ms`,
`quiet_since_ms`, `rallied`, `stand_until_ms` become stock-mind keys.

**Faults (D-UM-31).** The op budget; a call-level, depth, size or variable limit; a runtime type
error; a memory-shape violation; an intent overflow; an unknown contact or slot. A faulted think
lands nothing and leaves memory unchanged; the arena journals `EventKind::MindFault { unit,
think_seq, cause }` and increments `MindState.strikes`; a clean think resets them. Three consecutive
faults drop the unit to doctrine, the `HALTED` rule of `docs/help/scripting.md` applied to a hull.

**Doctrine (D-UM-32).** *Drops to doctrine* is defined once and names one thing: the hull's **drive
ROM**, Class 0 (§9), running **Standard Doctrine** (the stock mind, §11) at the hull's fallback
preset (Line unless the owner set another). It is not a component and cannot be destroyed; it holds
the helm, so a tripped or core-killed tank can still break off. The drop happens on the third
strike, on `destroy_core` (§9), at the seal when a licence has lapsed (§10), and is journalled
`EventKind::MindDropped` with a cause word. The pilot, owner and warrant principal read one line
(`Your core tripped: over its 1,500-op budget three thinks running. The tank is on the ROM; MIND is
off for the match.`); nobody else does. The deliberation layer's "mind link dropped" sentence
(`arena/ledger.rs:407`) is renamed `advisory link dropped` so the two vocabularies do not collide.
The ROM mind cannot fault: its ceiling is set at twice its measured worst case on the twin corpus and
that is asserted.

---

## 9. Hardware: the core, the classes, the pool (P5)

**The component (D-UM-33).** `core` is a new content kind, the ninth in `definitions/mod.rs KINDS`
and the seventh in `CONTENT_KINDS`, authored under `content/cores/` and installed as
`ComponentRole::Core { core }` beside `Weapon` and `Sensor` (`definitions/unit.rs:383`, lowered near
`:1386-1640`). It mints a capability `tacsim.core` with a minted rules schema the way `Power` does
(the only role that mints one today), carrying class, `think_ms`, ops, rows, `odds_rows`, slots,
memory keys, `line_to` cap, profiles and the governed bit. Check-out decodes it into
`UnitState.core: CoreProfile` as it decodes `SensorProfile`. A hull with no core component is Class
0: every hull has the ROM. Its `InstalledComponent.asset` is a titled object, so IC a core is its own
economic asset, separable from the hull.

**Placement and damage (D-UM-34).** Content places the core in a section: the turret on the line
tanks (fire control belongs there, and losing the turret loses the guns and the box in one line),
the chassis on the drone and the bipeds, the hull on the heavy. Two closed-vocabulary consequences
join D-16-9's list in both `Consequence` enums (`definitions/unit.rs:281`, `resolution.rs:153`):
`degrade_core` (one class lower for the rest of the match) and `destroy_core` (the mind drops to the
ROM at its next landing; continuous orders hold; a human aboard keeps manual control). Neither the
M32 revocation path (`wreck_ops` revokes only power, mobility and weapons) nor `remove_weapons`
(weapons only) does this today. Fitting a core IC is a refit work order; `UnitMutationOp` has no
install op, so M7 adds `InstallComponent` or rebuilds the unit from its spec, whichever the M32
review prefers (Q-UM-10).

**The classes (D-UM-35).** The three designs disagreed by three orders of magnitude; only the engine
angle's numbers fit the measured interpreter and the lens, and they are adopted. Think periods are
given at the 250 ms substep.

| Class | Name | Thinks every | Ops per think | Contact rows | Odds rows | Slots | Memory keys | Profiles | `line_to` |
|---|---|---|---|---|---|---|---|---|---|
| 0 | the drive ROM | 1,000 ms | 400 | 8 | 0 (the lock only) | 1 | 16 | 1 | 1 |
| I | the doctrine core | 1,000 ms | 1,500 | 12 | 2 | 2 | 32 | 2 | 2 |
| II | the line core | 500 ms | 3,000 | 16 | 4 | 4 | 64 | 3 | 4 |
| III | the command core | 250 ms | 4,500 | 24 | 8 | 6 | 128 | 4 | 6 |

Ceilings per unit per arena second: 400, 1,500, 6,000 and 18,000 ops, about 17, 63, 250 and
750 µs of CPU at 24 ops/µs. Class 0 runs only Standard Doctrine, at any slider values. The names are
lore (Q-UM-1): compute is scarce, so automation is excellent and people are decisive. The sim
chassis declare their cores in content: the line_tank family and both bipeds II, heavy_mk1 III,
scout_mk1 and drone_mk1 I (a scout is flown by its pilot), target_mk1 the ROM alone.

**The pool (D-UM-36).** Each arena has a deterministic compute allotment, the engine's guarantee that
no purchase can break the lens: `ops_per_arena_s = max(120,000, 500,000 − 1,000 × units)`, 200,000
at 300 units and 476,000 at 24. Admission counts class ceilings, never measured use. Recounted for
the harness's **300** machines: all at Class 0 take 120,000 and leave 80,000, about four Class III
upgrades at 17,600 net each; a 12v12 of Class III takes 432,000 and fits. The lobby splits the pool
evenly between sides at match setup and first-come within a side; `SIM MIND` and `SIM UNIT` check at
choice time and whichever comes second is refused with the arithmetic (`This side would think
96,000 ops a second; its half of the centre's pool is 100,000 and 92,000 are spoken for.`), so
`BOARD` never fails because of a core; check-out refuses an over-subscribed roster by name as the
last gate. Whether the allotment is a thing in the fiction is Q-UM-2.

**The sim (D-UM-37).** In the simulator a pod emulates the chassis's content-declared core; the
lobby lead may set the AI fill's class. Owned hardware matters only in IC engagements (which do not
exist yet). The economy angle's parity CR-2 is not adopted: the proving ground compares minds on
the chassis they will fly, and a lead who wants parity sets it (Q-UM-3).

---

## 10. The licence and the governor (P5, P6)

**Classes of mind (D-UM-38).** Three legal classes, enforced by construction: a class's engine
instance registers only that class's host functions, so a script that calls outside its class fails
at the seal. *Doctrine*: the Registry's published standing procedures, free, perpetual, open, loadable
on any core; at launch Standard Doctrine alone. *Discretion*: filed minds with bounded discretion over
their own hull's channels, the verb set of §4; licensed by term. *Frontier*: never mountable in a
hull; in play only as the deliberation lease. There is no Command class (D-UM-11, Q-UM-5).

**The licence (D-UM-39)** is a `mush-roll` record: holder (character or org), charter and line, pin
(a revision, or follow), seats (concurrent sealed matches), term on the `WorldIc` clock (at most 90
IC days, the Compact's audit term; nothing but Doctrine is perpetual), the warrant principal, form
and state (Active, Grace up to 3 IC days, Lapsed, Revoked by the Registry only, Recalled when the
revision failed). Renewal is a fresh auto-drafted E4 contract per term, so every licence payment
takes one path and one split. A lapse never reaches into a running match: the seal carries the state
at boarding; at the next boarding the hull drops to doctrine and only its own principals read why.

**Transferability (D-UM-40).** The owner's P6 is "traded or copied between players: yes definitely",
so licences are **transferable** instruments: a holder may sell on a licence they no longer want, the
transfer moves the entitlement and the seller loses it, and an author may mark a listing
non-transferable. `InstrumentKind::Entitlement`'s doc comment says non-transferable and nothing
enforces it; the comment is amended to "non-transferable unless the catalogue entry says
otherwise", and a mind licence is a catalogue entry that says otherwise. Sales and transfers between
characters of one account are refused through the lobby's `AccountId` oracle (alt laundering).
Charters trade too (§16).

**The governor (D-UM-41).** The governed bit in the core's capability payload. A governed core
refuses at the seal: an unfiled digest, a line with no live licence, a class beyond the licence, any
slot outside the lawful vocabulary or under the odds floor. The arena never reads a licence; it
trusts the seal as it trusts every other check-out fact. In the sim every pod's core is governed.

**Contraband (D-UM-42, built last).** Ungoverning a core is a refit at a facility without a
`core_integration` charter (the Tether's back room at Kirkwood); the work order exists in the
industry journal and the core's provenance records it. An ungoverned core seals unfiled drafts
(no fee, no levy, no Roll entry), runs above its licensed class, and arms unlawful slots: fire on
unclassified contacts, which beats W5's classification delay, or under the floor — the
**unwarranted act**, the setting's central crime (Q-UM-6). Heirlooms (pre-Retreat Enclave dies:
Class III performance, no governor) and greys (Martian dies off an unchartered line: no lineage) are
its hardware. Detection is evidence only: renewal attestation (an ungoverned core cannot attest),
custody inspection (deterministic, reads the bit and the provenance), the black box (W16: every IC
ledger opens with each unit's seal), and testimony (a hit by a contact the victim's own sensors never
classified). Consequences: seizure as a `Disputed` encumbrance then title to the Registry's principal,
a fine as an E4 obligation, revocation of the principal's licences for a term, Tycho voiding cover —
the everyday deterrent that needs no police. No contraband cut ships before its detection channel
exists.

---

## 11. The card: sliders, presets, profiles (P2)

The card is the parameter block a mind reads, declared by the mind as a schema from a closed widget
vocabulary — `slider(min, max, step, default, unit, help)`, `choice([...], default, help)`,
`toggle(default, help)`, `bearing(default, help)`, a facet set, a contact-class set — and rendered
by every client from that schema (D-UM-43). A value outside its range is refused with the range
named, never clamped. The stock schema below maps onto today's `ReflexConfig` (`config.rs:425-447`)
so that Standard Doctrine's **Line** preset on its defaults is today's machine — the regression
anchor. The player angle's mapping was corrected against the real defaults.

*Posture.* `stance` ∈ hold, close, orbit, hunt, screen (default hold; the bot's words plus the
`ORDER` objective's). `band_pct` 30–100 step 5, default 70 → `engage_band_permille` 700 (the card
prints the metres for the chassis: `420 m on line_tank`). `close_m` 100–400 step 50, default 200.
`orbit_sense` cw, ccw, auto. `turn_into_hits` on/off, default on, `turn_s` 2–10, default 4 (the
bot's rule). `screen_bearing` 0–359, `screen_arc` 60–180, default 120. `opening` wait, search,
advance, default wait (the bot's muster opening).

*Fire discipline* — the P1 levers, each a slot the stock mind arms. `min_odds` 10–95 step 5,
default 10, which is the Compact floor and today's behaviour (the reflex fires whenever ready with
line); `aspect` any, flank, rear, default any; `reserve_pct` 0–50 step 5, default 0 (today keeps
nothing back); `reserve_odds` 50–95, default 75; `ident` identified only — there is no `any` setting,
because the floor forbids it.

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
cannot show. Six: *Line* (the defaults above), *Sentry* (screen, min_odds 55, opening wait,
hunt_after 0, rally 0), *Sniper* (hold, band 100, min_odds 65, reserve 30, stop_unseen on),
*Skirmisher* (orbit, band 85, aspect flank, min_odds 35), *Hunter* (hunt, band 60, search 60,
hunt_after 45, min_odds 30), *Rearguard* (hold, break_off 70, rally 0, min_odds 50). Picking a
preset copies it into the profile; the card then says `Line, edited (min_odds 60)` until it matches
one again.

**Profiles.** A mind carries one to four named profiles by core class (§9), all sealed together. A
pilot switches between sealed profiles with `MIND PROFILE <name>`, a recorded intent at most once per
10 s: switching is not an edit, so P4 and replay hold.

---

## 12. The pilot's flow: choosing, sealing, boarding, releasing

**In the lobby.** `SIM MIND` beside `SIM UNIT` in the style of `match_/sim.rs`'s selectors, during
selection only. Bare, it prints what the slot will carry and what in the library fits the chassis's
core (`Yours: none. Fits line_tank (II): Standard Doctrine, Sentinel 2.1 (licensed), mine r7.`).
`SIM MIND mine [PROFILE Sniper] [@r7]` sets it; with no pin the latest saved revision is resolved at
each `SIM START` and printed then. `SIM AI MIND Sentinel PROFILE Sniper` is the side lead's: it sets
the mind the side's AI fill runs, default Standard Doctrine on Line, which is today's machine. Fit
(class, rules, profiles, the pool) is checked when the choice is made; the later of `SIM UNIT` and
`SIM MIND` is the one refused, so `BOARD` never fails because of a mind. While selection is open
everybody reads which mind a slot will carry, as with `SIM UNIT`; once the match starts the board
stops printing anybody's mind, for `SIM UNIT`'s reason.

**Sealing (P4).** At `BOARD` for a pilot, at `Boarding → Running` for the AI fill, the pin, the
whole card (every profile and its rules), the core, the licence class and state, the warrant
principal and the channel set are frozen into the seal (§19). The cockpit says so in one line before
`STARTUP`. Editing a mind in another window during the match makes a new revision and changes
nothing aboard.

**What a mind holds in v1, and the T-54 boundary.** The three designs all assumed a pilot and a mind
could split the tank's channels (ASSIST: the pilot drives, the mind locks and fires). That needs
per-channel K6 leases (16c T-54, unbuilt) and a `(unit, channel)` epoch the mind owns. V1 therefore
offers two modes only: **NONE** (the sealed mind is dormant) and the **whole-unit mind**, which flies
AI fill, released tanks and proving matches and holds all three channels. P1's co-piloting is still
delivered to a seated pilot in v1 — not through the mind but through the core's condition slots,
which are native and evaluated for a piloted unit exactly as for a machine (§5): the pilot drives,
the core fires under the pilot's armed slots. `ASSIST` and `MIND ON|OFF <channel>` arrive with the cut
that gives a unit a second station (§22, after M8).

**Release and possess.** A pilot who goes linkdead keeps their continuous orders and hands nothing
over (today's rule); once `Released` past the grace (or on `EXIT`, or a revoked lease) the tank goes
to its sealed mind, and the board reads `flown by its mind`. `BOARD` takes it back as today. Possess
and act-as are the existing seat path: `mush_sim::request_seat_activation`, then `apply_control`
raises the `(unit, channel)` epochs and calls `set_controller(Pilot)`, which refuses in-flight
thoughts through `Refusal::Epoch`. New behaviour: `set_controller(Pilot)` does **not** delete the
`MindState` row (the reflex's `release` deletes its row today); the mind's memory, conditions and
notices stay dormant, and a release adopts the mind again with its memory intact.

**The cockpit `MIND` verb** (the fifteenth selector, `GOTO` the sixteenth). Bare: a status block of
at most seven lines under 120 columns — mind and seal, core and class, profile, who holds each
channel, ops at the last think against the budget, each rule's truth at that think, the last
decision. `MIND PROFILE <name>`; `MIND RULES`; `MIND TRACE [n]` (the last n thinks, default 3, one
line each plus intents). On the TUI and the web cockpit screen a MIND block of six rows and at most
40 columns joins the right-hand column under the weapons block; the map keeps its 78 columns; the
first three rules carry a state glyph; clicking a rule on the web prefills `MIND PROFILE …` and sends
nothing until Enter. The card draws the same way everywhere, one slider a line with a 10-cell bar and
the unit-aware value; a rule shows its English under it.

---

## 13. Records, the seal and replay

**The seal** is computed world-side at check-out, in order: the hull's loadout; the licence state;
the core's class, governed bit and condition; the revision (a follow pin resolves to the newest
revision filed before boarding); the card validated against the schema, refused not clamped. It
holds: unit, core component, class, governed bit; charter, line, revision; `source_sha256`; the
canonical card bytes and their digest; the armed slots; `MIND_ABI` (`rhai=1.26.0;api=N;stock=M`);
the licence id or `doctrine`; the warrant principal; the channel set (all three in v1); the fallback
reason if any. It lives on `CustodyUnit` beside `sensor`, `weapons` and `gunnery`, for every unit,
piloted or not.

**Where it goes.** `MatchRecord` (`record.rs:334`) gains `minds: BTreeMap<ObjId, MindSealRecord>`;
the ledger opens with one `EventKind::MindSealed` line per unit (unit, mind id, the first 16 hex of
the source hash, card digest, class, governed bit, licence validity, warrant principal), so the
ledger digest attests which minds fought and W16's black box carries the seal. Sources live immutably
at `tacsim:mindsrc:<sha256>`. `state_hash` (`hash.rs:485`) folds the `MindState` rows after the
reflex rows (`:593`) in unit-id order: seal digest, `think_seq`, `pending_think_seq` and
`pending_t_ms`, `think_epochs`, memory digest, the range map, the notice buffer, order, strikes,
dropped, the condition slots, the `MotorRow`; an `each_new_row_field_moves_the_state_hash` test as
`reflex.rs:5911` does. The snapshot (format 2) carries the rows whole and the seals by reference,
and resume refuses closed when `MIND_ABI` or the rhai version differs, the precedent being
`SnapshotError::Physics`.

**Replay** here means re-simulation, not cut I's picture. The check-out snapshot, a **seat-intent
tape** and the seals reproduce the per-substep hash sequence. M6 persists the tape
(`tacsim:tape:<match>`) for OOC matches, under its own bound `[tacsim.minds].tape_max_bytes` and the
centre's `HISTORY_DEPTH` of 20, so the bench can step any think of any kept match. The player angle's
per-unit picture tape is not kept: at 300 units it is a storage bomb, and the seat tape plus
determinism regenerates any picture on demand. A replay across an engine version is refused in words
(`Match 31 was thought on mind engine 1; this server runs 2.`).

---

## 14. The REPL, the proving ground and the bench (P7)

**The trace ring.** Per mind unit, in arena RAM, outside the hash like `last_frames`: the last 64
thinks — `think_seq`, `t_ms`, ops, fee, the landed intents with their outcomes, the rules with their
conditions' values (`R1 FIRE cannon: odds 54 >= 60 false; aspect side true -> no`), each kernel
called and its fee, the author's `trace()` lines, any fault. It egresses on a new `arena.mind` feed
at most once per arena second, only to the unit's principal (the record's licensee, owner and warrant
principal), the side lead for their own AI fill, and staff. A mind's trace says what its platform
saw, so nobody else may read it: the observer rule applied to the debugger. For a sealed-form mind
you bought, the trace shows its rules, card, intents, ops and the author's `trace()` lines, never
`mem` or source.

**The proving ground** is the bot's lanes made a verb: `@mind/prove mine/line VS Standard/Hunter 30
MUSTER 1v1`. The opponent is any Doctrine preset, any mind in your library, `mirror`, or a published
mind whose seller turned sparring on (results, never internals). Scenarios DUEL, MUSTER (the figure
of record), HOLD; sizes 1v1 to 6v6. N is 10–100 and even: each of N/2 seeds flown once from each
side. Proving matches are headless arenas, deterministic, no wall clock, run unthrottled on
`[tacsim.minds].proving_workers` (default 1, one run per account at a time), never sharing a worker
with a live match; a harness lane asserts two running proving arenas leave the live p99 where it was.
The quota is unit-minutes per account per day (default 2,000). The report reuses the bot's lines in
the game's voice: won, lost, drew, timed out as its own column; `win rate 19 of 30: 95% Wilson 46 to
78%`; `lost the first section in 11; won 2 of those`; shots by weapon and odds bucket; rules fired
and held; hits by facet; rounds left; ops p50 and p99; faults; the seeds. Two minds on the same seeds
print `difference called` only when the intervals do not overlap. `wilson_95` moves from
`mush-bot/tests/support/arena.rs:1439` into a tacsim library module outside `arena/` (it is f64).

**The bench** is a booth in the simulator centre with one verb, `BENCH`, rendered on screen 0 with
the cockpit and the MIND block. A **closed-loop** session (`BENCH LOAD 14/7`) re-runs proving match 7
of run 14 from its seed under a manual clock, which needs no tape because no human was in it: `STEP
[n]`, `RUN TO 1:30`, `BACK [n]` (re-simulate from the start; cheap and exact), the hash shown at every
step and matched against the run's. An **open-loop** session (`BENCH LOAD MATCH 31 RA`) re-simulates
a kept OOC match from its seat tape to a think and answers "what would this mind decide given this
picture", never "what would have happened". In either: `PIC`, `TRACE`, `WATCH <expr>` and `EVAL
<expr>` (read-only over `pic`, `card` and `mem`, in a sandbox that emits no intent, on a separate
small budget), `DIFF <mind>` (a second mind over the same pictures, listing the thinks where intents
differ), `SWAP <mind>` (forks the timeline and says so), `POSSESS` and `RELEASE` (your cockpit verbs
drive the unit under the manual clock), `ACT AS <callsign>` (any unit of a private run, the opponent's
included). This is the NPC-REPL possess-and-act-as idea on a deterministic substrate. Nothing in the
bench touches a live match. Rhai's own `debugging` feature (statement stepping) is deferred until its
overhead on the live interpreter is measured, because features unify across the workspace.

---

## 15. The author's workflow

A library belongs to a character (minds are IC goods); in the sim an account may fly any mind any of
its characters holds. One verb, `@mind`, carries the workshop: `@mind/new <name> [FROM Standard|
<mind>]`, `/list`, `/info`, `/source <name>[@rev] [from-to]`, `/paste` (lines until a lone `.`),
`/line`, `/insert`, `/delete`, `/check` (compile; run on three canned pictures — nothing in sight,
one hostile at 450 m, hurt with two hostiles; report line and column, ops per think, intents, class
violations), `/save <note>`, `/card`, `/set <name>/<profile> key=value`, `/preset`, `/profile
add|del|rename`, `/rule <name>/<profile> n=<text>`, `/fork` (open minds only, lineage recorded),
`/diff`, `/prove`, `/publish` (§16). Revisions are immutable and content-addressed:
`DefinitionDigest::for_canonical_value` under domain `tacsim.mind.v1` over the normalised source, the
schema, the presets, the class and `MIND_ABI`; its first six hex digits are the seal id players see.
Named versions are kept while you own the mind; the last 50 unnamed revisions, plus any a kept
record, run or listing references. Sources are capped at 32 KiB, a library at 20 minds.

Three editors, each first-class for a different author. The web workshop is a Svelte page: the
library, a CodeMirror 6 editor with Rhai highlighting and inline compile errors from a `mind.check`
request, the card rendered from the schema, the rules with their English, a Try panel with the
canned-picture trace, Prove and Bench buttons. Over SSH, `mind push|pull|check` through a narrow
exec allow-list with public-key auth only, which amends design 02 §5.6's "interactive shell only"
(`mush-net-ssh/src/server.rs:1550`) and is Q-UM-7. The TUI edits cards and rules and views source
read-only.

The raw tier: one script with a header `const MIND = #{ name: "Sentinel", api: 1, class:
"discretion", core: "II" };`, an optional `fn card()` returning its schema, and `fn think(pic, card,
mem)`. The stock library is native kernels with published fees: `stock::think`, `stock::rules`,
`stock::fire_discipline`, `stock::best_hostile`, `stock::hold_band`, `stock::steer_to`,
`stock::hunt`. An author's first real mind is usually three lines calling `stock::think` and
overriding one decision. `help minds` (pilots), `help mindcraft` (authors; the API, the fees, the
classes, the seal), `help proving`. Two tutorial hint points join the five in `tutorial.rs`
(`HintPoint`): after the first finished match (`SIM MIND Standard lets your tank's core run the
Registry's doctrine by your rules`), and on the first `@mind/save` (`@mind/prove <mind> VS Standard
flies it thirty times and tells you how sure to be`). A new character's library holds Standard
Doctrine, open, with teaching comments and the six presets; their slot carries no mind until they
choose one; the fill they fight is Standard on Line.

---

## 16. The Roll and the market (P6)

**Filing.** Publishing a mind files a revision on the Registry's Roll of Minds — the Registry being
already the notary of last resort (doc 11 §2). Filing runs conformance (compiles under its class's
engine; the schema validates; at most 8 presets; the slots are lawful; the reference bench rates the
revision's worst observed ops per think on the Registry's fixed scenario set, so a revision that
overruns a Class I core is rated "II and above"), charges a fee to `system:tacsim.registry`, and
refuses a normalised duplicate of another charter's revision in one sentence that names nothing.
Behaviour is not protected; a clean-room mind that plays the same way is legal, and that competition
is the market. The Roll entry is public (name, house through the hidden-org renderer, class, lines,
rated profile, forms offered); the **charter** is a world object titled through `issue_title` by the
Registry's principal. Selling the mind is a title transfer; revenue follows title; a charter can carry
a `Lien` or serve as `Collateral`; stealing the object conveys nothing (doc 08 D5). A line is a
compatible schema; revisions within a line are free updates; followers seal the newest revision filed
before boarding, pinned holders keep theirs; an author may withdraw a revision, only the Registry
recalls one; a rhai bump re-runs conformance for every live revision and recalls failures with
notice.

**Listing and sale.** A filed line is a `mush-market` listing of `Representation::Entitlement`
under catalogue ruleset `minds`, reached through tacsim-registered `CatalogSource`,
`AvailabilityProvider` and `DeliveryProvider` tables; `mush-roll` takes no market edge, and the
public projection gate holds. `@mind/publish Sentinel@2 price=400 terms=sealed seats=6
sparring=on`. Terms: *sealed* (the default: buyers run and tune it, read its card, presets, rated
profile, notes and probe-level traces, never the source), *open* (source and fork rights; forks file
as derivatives naming their parent; the parent's royalty, capped at 300 per mille with a chain depth
of 3, is paid inside every derivative's settlement), *open-resale* (forks may be published; the
listing shows `derived from Sentinel by Ada`). A purchase drafts an ordinary E4 contract with escrow;
settlement is one balanced `PayoutShare` split: the Compact levy (seed 50 per mille) to the
Registry's sink, the venue fee when a vendor org lists the line, the parent royalty, the remainder to
the charter owner's account. The `DeliveryProvider` writes the licence; payment never edits a
loadout. A buyer follows the major version unless they pin; a new major is a new listing.

**The vending machine is one seller among players by construction.** NPC houses write minds (staff
authored), file them on the Roll under their NPC org exactly as a player does, pay the fee and the
levy, and sell through a `mush-market` `Vendor` with budgets. The launch catalogue: the Stickney
Yards Doctrine Library (Standard Doctrine's preset pages, free); the Meridiani Field Series
(Discretion, sealed, the mercenary standard); Tycho Mutual's Safe Conduct (convoy defence; Tycho
discounts cover on hulls that run it); the Territorial Force's Coprates Drill (Republic-standing
buyers only, through an `AvailabilityProvider`). Later, NPC procurement buys from the Roll on a
monthly budget by the Registry's trial rating per credit, with a standing filter and a diversity cap
of 40 percent of an org's hulls per charter (Q-UM-8), which is how a player who only writes AI sells
to factions and what passes the zero-PC test.

**Hardware IC.** Dies reach the corridor as `tacsim.electronics` lots graded `die.c1|c2|c3` with
`Lineage::Origin` from a chartered importer (Basin Charter Instruments: a Luna depot, agents at Ceres
and Phobos, an allotment per IC month, which makes Class III scarcity a world fact). Stickney
integrates them (`core_integrate@1`); fitting is a refit work order. Prices relative to a line tank's
catalogue baseline: a Class I replacement 3 percent, II 8 percent, III 25 percent and
`Legality::Restricted`, sold only to a principal holding a `tacsim.licence.core3_end_user`
entitlement, checked at transaction time the way `extraction_authorized` checks an extraction
licence. Sim matches never post to the ledger (D-16-3); buying a licence is an IC market
transaction outside the match (Q-UM-4).

---

## 17. Migrating `reflex.rs` into Standard Doctrine, and the twin fixture

**What moves.** The state machine and its edges (`health_permille` under threshold → Withdraw;
`is_dry` → Withdraw for good; the rally once after `rally_ms`); the utility score in
`best_hostile_where`; Patrol's goal precedence; Approach to `stand_off_cell` at `stand_off_want`;
Engage (lock, fire when ready with ammunition, power and no overheat, gated by line); the Screen and
Support orders with `SUPPORT_LEASH_CELLS` and `SUPPORT_STATION_CELLS`; standing-order obedience and
expiry. What the port needs is exactly §3 to §5: the view with aspect, odds, `reads_dead` and the
order; memory for the clocks and the state; the kernels `best_hostile(w_range, w_facet, w_health,
prefer)`, the nav pickers, `line_to(contact)`; the verbs with `goto`, `hold` and `arm`.

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
memory bytes over a corpus of (view, memory) pairs harvested from 30 seeded authored-cohort runs; on
the cross-tempo fixture with two mind units (one event list, one hash sequence at 5,000, 1,000 and
500 ms); and on whole-match per-substep hash sequences. *Gate B* bounds the divergence: T0 against
T2 over 30 pooled matches with sides swapped; the 95 percent Wilson interval on T2's win rate must
contain 50 percent; shots per minute, time to first shot, the state distribution
(`ReflexMemory::distribution`), path queries and corner refusals reported against T0. If B fails the
Line sliders are retuned until it passes before the default flips. After the flip `Controller::Reflex`,
`ReflexTick` and the decision half of `reflex.rs` are deleted and its motor half lives on as
`motor.rs`; `NotReflex` becomes `NotMachine` then.

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
(each a fire trace); `odds_rows` per class is the lever if it is large. The pool's ceiling at 300
units, 200,000 ops per arena second, is 100,000 per window: **4.17 ms** at 24 ops/µs. The engine
angle's 4 ms CPU lens would therefore fail its own ceiling lane by construction; the lens is set at
**5 ms per window on both cohorts**, `[tacsim.minds].cpu_lens_ms`, separate from the window lens
(think time is off the critical path at Engaged) and inclusive of whatever ran inline. The headroom
against the window lenses stands at the baseline: CI window p99 10.56 ms against 12, authored
14.1–16.3 ms (18 an upper estimate) against 20, wakes 6.55 and 7.7–8.2 ms against 12. dev1's
ops/µs is unmeasured and may be markedly lower than ALIEN18's; the op budget is deterministic
regardless, the lens is wall time on whatever host runs it, and the pool is a config knob.

**The lane.** `tacsim_harness_arena_minds` on the existing ignore string, the
cohort checked out through `tacsim::match_::custody_unit` as `Controller::Mind`, three variants on
the production composition: `minds_stock_ci` (the CI cohort, every machine on Standard at Class 0),
`minds_stock_authored` (the authored cohort, the same), `minds_ceiling_authored` (the pool filled
to its admission ceiling by a synthetic Class III load mind that spends exactly its budget and emits
a legal intent set). The lane runs the task's real policy — wakes at `next_due_ms`, the batch on a
2-worker pool awaited before the next wake — with `TACSIM_WAKE_JITTER_MS` forcing multi-substep wakes
on a seeded fraction so the inline path is measured. Under `TACSIM_ASSERT` and through
`scripts/harness-weekly.sh` it asserts: wake p99 ≤ 12 ms on both cohorts; window p99 ≤ the cohort's
lens counting inline think time; mind CPU per window p99 ≤ 5 ms; zero faults on the stock variants;
ops per substep, peak and total, within the pool. Its deterministic block (thinks, ops, fees, faults,
landings, condition fires, the final substep hash) joins the delta test against the committed
baseline. Reported, not asserted: `view_us`, measured ops/µs, think µs p50 and p99, `pool_wait_us`.

**The fast path has one trigger.** If `minds_stock_authored` misses a lens on the blessing host
after the custom-type view and the kernels are in, T1 is promoted to production for Standard
Doctrine at any slider values; its seal record says `engine native-stock`, and the Rhai stock mind
stays the exact reference it is held to by Gate A. Raw-Rhai minds are always interpreted. No other
fast path is admitted.

---

## 19. Fit with the crates

`crates/tacsim/src/arena/{mind.rs, motor.rs, view.rs}` (library, no Rhai, no tokio);
`crates/tacsim/src/minds/` (the API, kernels, `RhaiRunner`, `content/minds/standard.rhai`);
`crates/mush-mind` (the sandbox; depends on `rhai` alone); `crates/mush-roll` (drafts, the Roll,
charters, licences, loadouts, seal records; depends on `mush-core`, `mush-org`, `mush-economy`,
`mush-mind` for conformance; no `mush-script`, no `mush-market`, no `tacsim` edge; the market reaches
it through tacsim-registered tables). Against 08i §4 rule 1: drafts, Roll entries and charter
metadata are bounded whole-document workflow records (exempt, strict decoding, deterministic keys);
licences, loadouts and seals decide which code runs and which verbs a hull may emit — action legality
— so they are authoritative typed families on the prepared path from birth. One new Entitlement
instrument, `tacsim.licence.core3_end_user`; two system principals, `system:tacsim.registry` and
`system:tacsim.sim`. A `FactProvider` publishes `minds.licence.lapsed`, `minds.filed`,
`minds.seized`, `minds.unwarranted` so contracts and fronts can condition on them. `[tacsim.minds]`
holds the classes, the pool formula, `cpu_lens_ms`, `workers`, `proving_workers`, the quota, the
odds floor, the levy, the fees, the term cap, `tape_max_bytes`.

Reviewed diffs this proposal knowingly makes: `LABEL_KINDS` 5 → 6; `COCKPIT_VERBS` 14 → 16;
`FRAME_VERSION` 1 → 2 with the bot parser and the mush-ansi goldens; `Refusal` gains `Unwarranted`;
`not_reflex` → `not_machine` at the deletion cut; `ReflexCounters` → `MotorCounters` moves the
baseline keys; rhai pinned exactly at the workspace; `InstrumentKind::Entitlement`'s doc comment.

---

## 20. Decisions (proposed; binding when the owner answers §21)

| # | Decision |
|---|---|
| D-UM-1 | A mind is a pure function `mind(view, memory, card, keyed draws) -> (≤16 intents, memory')` run through `trait MindRunner` in `arena/mind.rs`, which names no Rhai type; the interpreter lives in `crates/mush-mind` (rhai only) and `crates/tacsim/src/minds`; the registry is `crates/mush-roll` with no `mush-script` edge. |
| D-UM-2 | `Controller` gains `Mind` (hash code 2); every unit without a seated pilot is `Controller::Mind`; `Controller::Reflex` and `ReflexTick` are deleted after the twin gates pass. |
| D-UM-3 | `reflex.rs` is split: the decision half becomes Standard Doctrine in Rhai; the driving half stays native as `arena/motor.rs` on `DueKind::MotorTick` with every path bound unchanged. The twin fixture has three columns (T0 shipped reflex, T1 native oracle, T2 Rhai); Gate A is T1 ≡ T2 exactly; Gate B bounds T0 vs T2 by a 30-match pooled Wilson interval containing 50 percent; leaks are never restored. |
| D-UM-4 | The parity rule: a mind reads exactly the `arena.cockpit` keyframe's content, built by `Arena::mind_view` from the frame builders; a tier truncates and never extends; every mind field is first a `Frame` and keyframe field with a `FRAME_VERSION` bump. The cockpit row formatter moves into `arena/view.rs`. |
| D-UM-5 | Aspect (`facet_band_seen` off the plot), per-weapon odds for classified contacts only (`resolve_seen` against a new `Arena::aim_at`, limited to the core's `odds_rows`), `reads_dead` derived from the own kill or the held fatal reading (never `contact_reads_dead`), the standing order (re-keyed to the unit's own `ContactId`) and `t_ms` join the frame and the keyframe for pilots and minds alike. No course estimates. |
| D-UM-6 | Contacts are named in the API by the opaque `ContactId`, with designator and alias as fields. |
| D-UM-7 | The view's Rhai types are registered custom types with getters, never maps (15.05 vs 24.69 µs). |
| D-UM-8 | `MindState.notices` is a bounded (16), hashed, snapshotted buffer filled by the post-hash journal scan and drained per think; the mind's own refusals are appended to it. |
| D-UM-9 | A think is atomic: at most 16 intents into a native buffer; a fault lands nothing and keeps memory. |
| D-UM-10 | The verb set stays closed in both directions: a mind has no verb a pilot cannot type, and every added intent gets its cockpit verb in the same cut. `COCKPIT_VERBS` grows to 16 with `GOTO` and `MIND`. `IntentKind::Goal(MotorGoal)` on Helm drives the native motor for pilots and minds; `IntentKind::Condition` on Fire arms a slot for both. |
| D-UM-11 | A mind never issues `ORDER`, `RELEASE`, `PLACE`, `BOARD` or `EXIT` (the Warrant: no automated system commands); refused with `Channel`. No Command class. The deliberation layer's `ORDER` path is unchanged. |
| D-UM-12 | Landing is `Arena::apply_machine_intents`, the factored body of `Arena::order`, with an Emcon arm calling `resolve_emcon` added, resolving at the landing millisecond through `apply_intent` and the existing resolvers. |
| D-UM-13 | Landed intents carry the epochs recorded at collection; a reclaimed seat refuses stale thoughts with `Refusal::Epoch` through the exact-epoch gate. |
| D-UM-14 | `IntentSource::Mind` joins `Seat` and `Machine` for attribution; it is not counted on the per-unit seat window; condition fires are journalled `by: cond`. |
| D-UM-15 | Squad orders reach `Mind` units through the unchanged `Order` path; the commander's hint and `Support{of}` are re-keyed at write time through `draw_contact_id` to the unit's own held `ContactId` or none; the Support branch's read of the charge's row is dropped. |
| D-UM-16 | Condition slots (`FireCondition`: weapon, target Lock / Contact / BestHostile, `min_permille`, aspect mask, `min_rounds_left`, `max_range_m`), up to the core's slots, hashed, evaluated natively at `WeaponReady`, at a sensor pass refreshing the target's plot and at landing, firing through `apply_intent` and `resolve_fire` at no mind cost; pilots get the same slots from the same core via `FIRE <weapon> WHEN …`. |
| D-UM-17 | The fire law: every mind-issued `FIRE` cites a slot (`fire(slot, condition)`, evaluated at landing); a citation that does not hold is refused `Unwarranted`, a new `Refusal` word. The legal floor is validated at the seal: no slot names an unclassified contact; every slot carries at least `odds_floor_permille` (seed 100). There is no free mode; "weapons free" is the labelled `BestHostile`-at-the-floor SOP. A pilot's typed `FIRE` is never gated. |
| D-UM-18 | The rule language `<ACTION> [weapon] WHEN cond [AND cond]…` (≤4 conditions, no OR, English written back) is the authoring surface; rules that fit a slot compile to one, the rest are evaluated by the stock mind at its think; rule actions are `FIRE`, `HOLD FIRE`, `LOCK …` only. Precedence: an `ORDER`'s hold-fire, then `HOLD FIRE` rules, then first match per channel, then sliders. |
| D-UM-19 | An `ORDER`'s `hold_fire` is enforced in `resolve_fire` for `IntentSource::Mind` and condition fires (new resolver code). |
| D-UM-20 | A think is `DueKind::MindThink` at a lattice millisecond (`stagger_ms(seed, unit, think_ms)` plus multiples), resolved in the substep its timestamp falls in after that millisecond's other due items; the view is the state then, with the notices scanned up to the previous substep's end. Never a pulse. |
| D-UM-21 | The landing is `DueKind::MindLand` at `t_ms + substep_ms` (the match's substep as read at collection), in unit-id order then emission order. The cross-tempo fixture with two mind units is a gate. |
| D-UM-22 | Thinks run on a process-wide pool (`[tacsim.minds].workers`, default 2) shared by every arena task; at a single-substep wake the batch runs after egress and is awaited before the next step; when the landing falls in the same wake the runner is called synchronously at the landing point inside the watchdog. A batch is bounded by the pool ceiling and never skipped. |
| D-UM-23 | A snapshot carries no job; `MindState` carries `pending_think_seq` and `pending_t_ms`, and resume recomputes the job; a fixture snapshots between N and N+1 and asserts the hash sequence. |
| D-UM-24 | The sandbox is one shared `Engine::new_raw()` with Core, Arithmetic, Logic, Iterator, Array, Map and BasicString packages only; no time, no math; `eval`, `import`, `export`, `Fn`, `call`, `curry` disabled; strict variables; `DummyModuleResolver`; `on_print` and `on_debug` to the trace ring. rhai pinned exactly `=1.26.0` at the workspace; `MIND_ABI = rhai + api + stock versions`, recorded per match; a rhai upgrade is a reviewed ABI bump. |
| D-UM-25 | Limits: call levels 12, expr depths 48/24, string 128, array 64, map 64, variables 64, functions 32, modules 0, anonymous functions off; `max_operations` at the class ceiling as a backstop only. |
| D-UM-26 | The budget is ops plus kernel fees checked in `on_progress` with no clock anywhere; fees calibrated on the blessing host at about 42 ns per op; every think pays a fixed 20. |
| D-UM-27 | The card is injected as a read-only custom type in scope; one AST per `(source_sha256, MIND_ABI)`, cached. |
| D-UM-28 | Floats may appear in expressions; no float function is registered; memory values and intent arguments are INT only (a FLOAT is a type fault); a Windows-vs-Linux hash-equality CI test over the twin corpus is mandatory. |
| D-UM-29 | Mind randomness is a sixth `DrawLabel`, `Mind { unit, think_seq, n }`; no stream, no other entropy. |
| D-UM-30 | Memory is a bounded canonical map (per-class key cap, keys ≤24 chars, values INT/bool/unit/string ≤32/array ≤32 INTs); its digest is folded into `state_hash`, its bytes carried by the snapshot; the arena holds bytes, never a `rhai::Map`. |
| D-UM-31 | A fault journals `MindFault` with a cause word; three consecutive faults drop the unit to doctrine. |
| D-UM-32 | "Drops to doctrine" means the hull's Class 0 drive ROM running Standard Doctrine at the hull's fallback preset; it holds the helm, cannot be destroyed and cannot fault; journalled `MindDropped`. The deliberation sentence becomes "advisory link dropped". |
| D-UM-33 | `core` is a new content kind and `ComponentRole::Core`, minting capability `tacsim.core` with a minted rules schema as `Power` does, decoded at check-out into `UnitState.core: CoreProfile`; a hull without one is Class 0; the component's asset is a titled object. |
| D-UM-34 | Content places the core in a section (turret on the line tanks, chassis on drone and bipeds, hull on the heavy). Two new closed consequences, `degrade_core` and `destroy_core`, in both `Consequence` enums. IC fitting is a refit work order needing a new `InstallComponent` op or a rebuild from spec. |
| D-UM-35 | The classes are 0 (1,000 ms, 400 ops, 8 rows, 0 odds rows, 1 slot, 16 keys, 1 profile), I (1,000 ms, 1,500, 12, 2, 2, 32, 2), II (500 ms, 3,000, 16, 4, 4, 64, 3), III (250 ms, 4,500, 24, 8, 6, 128, 4); lore facts in `[tacsim.minds]`, the harness the gate. Chassis declare their cores in content. |
| D-UM-36 | Each arena has a deterministic pool `max(120,000, 500,000 − 1,000 × units)` ops per arena second, admitted by class ceilings, split evenly per side, first-come within a side, checked at `SIM MIND`/`SIM UNIT` (the later choice refused with the arithmetic) and at check-out by name. |
| D-UM-37 | In the sim a pod emulates the chassis's content-declared core; the lead may set the AI fill's class; owned hardware matters only IC. |
| D-UM-38 | Three legal classes enforced by registered host functions: Doctrine (Registry only, free, perpetual; Standard Doctrine at launch), Discretion (own hull, licensed by term), Frontier (never mountable). |
| D-UM-39 | A licence is a term lease (≤90 IC days) in concurrent seats naming a warrant principal, with Active, Grace (3 IC days), Lapsed, Revoked (Registry only), Recalled; renewal is a fresh auto-drafted E4 contract; a lapse never touches a running match and drops the hull to doctrine at the next seal, told only to its own principals. |
| D-UM-40 | Licences are transferable unless the author marks the listing otherwise; `InstrumentKind::Entitlement`'s doc comment is amended to say so for catalogue entries that permit it; sales and transfers between one account's characters are refused through the lobby's `AccountId` oracle. Charters trade as title transfers. |
| D-UM-41 | A governed core refuses at the seal an unfiled digest, an unlicensed line, an over-class mind and an unlawful slot; the arena never reads a licence; every sim pod is governed. |
| D-UM-42 | Contraband is ungoverned cores (an unchartered refit leaving provenance; heirlooms; greys) detected only by evidence (renewal attestation, custody inspection, the black-box seal lines, victim-ledger testimony) with seizure, fine, revocation and Tycho voiding cover; no contraband cut ships before its detection channel, and none before IC combat. |
| D-UM-43 | The card is a mind-declared schema from a closed widget vocabulary, refused not clamped; the stock schema maps onto `ReflexConfig` so Standard Doctrine's Line preset on its defaults is today's machine (withdraw 35, stand 17.5, search 60, weights 100/40/30, min_odds at the floor, no reserve). |
| D-UM-44 | Six presets (Line, Sentry, Sniper, Skirmisher, Hunter, Rearguard) are named cards only; one to four profiles by class, sealed together, switched by `MIND PROFILE` as a recorded intent at most once per 10 s. Pilot-plus-mind channel splits (ASSIST, `MIND ON|OFF <channel>`) are deferred behind T-54; v1 modes are NONE and the whole-unit mind, and a seated pilot's SOP fire comes from the core's native slots. |

---

## 21. Open questions for the owner (recommended defaults marked)

1. **Q-UM-1, names.** *Standard Doctrine* for the stock mind; the *drive ROM* for Class 0; *doctrine
   core*, *line core*, *command core* for I–III; *Discretion* for the licensed class; the *Roll of
   Minds*; *Basin Charter Instruments* for the importer. **Default:** names ok; list replacements.
2. **Q-UM-2, the pool in the fiction.** **A (recommended):** visible — the centre's compute is
   finite, `SIM MIND` prints the arithmetic, and IC the arena's allotment is a resource an operator
   sells or rations so it can be fought over. **B:** an invisible engine cap that refuses by name.
3. **Q-UM-3, sim hardware.** **A (recommended):** a pod emulates its chassis's declared core and the
   lead may set the fill's class. **B:** every rated sim lane runs parity Class II regardless of
   chassis, so the proving ground compares minds and never wallets.
4. **Q-UM-4, IC money for OOC sim use.** **A (recommended):** sealing another author's mind in the
   sim needs a licence bought with IC money on the market (the one place IC money crosses toward the
   sim, as an IC good the character holds; the match itself still posts nothing, D-16-3); Standard
   Doctrine and sparring are free. This gives a mind-smith a market before IC units exist. **B:**
   the sim is practice: anyone flies any published mind, licences gate IC hulls only.
5. **Q-UM-5, may a mind ever ORDER?** **A (recommended):** never; the Warrant's "no automated
   system may command" is a hard lore fact, and the only automated orders are W7's advisory lease
   through a human-warranted squad. **B:** a Command class on a Class III core, a character as
   principal, inside W8's span and rate, `Legality::Restricted`, as end-game content.
6. **Q-UM-6, contraband autonomy.** Given P1's "no auto-fire as a feature": **A (recommended):**
   an ungoverned core may arm unlawful slots — fire on unclassified contacts or under the floor — as
   the setting's central, rare, provable crime; the ROM and every governed core never can. **B:**
   ungoverning only skips the Roll and class checks and the floor is never lifted.
7. **Q-UM-7, SSH push and pull.** **A (recommended):** amend design 02 §5.6 with a narrow exec
   allow-list (`mind push|pull|check`), public-key auth only. **B:** SSH authors stay on `@mind/paste`
   and the web editor.
8. **Q-UM-8, player minds on NPC armies.** **A (recommended):** NPC procurement buys from the Roll
   (budget, trial rating per credit, standing filter, 40 percent diversity cap), so one player's code
   may fly the Republic's machines against other players. **B:** NPC houses run staff minds only.
9. **Q-UM-9, floats in scripts.** **A (recommended):** allowed in expressions, INT at every boundary,
   behind the cross-platform hash CI test. **B:** rejected at the seal by an AST walk (may need the
   `internals` feature), which makes the language integer-only.
10. **Q-UM-10, the install op.** **A (recommended):** add `UnitMutationOp::InstallComponent` to
    `mush-tactics` under the M32 plan/apply/journal discipline. **B:** a refit rebuilds the unit from
    its spec and re-applies condition.
11. **Q-UM-11, the T-54 cut's place.** **A (recommended):** per-channel leases and ASSIST land after
    M8 as their own cut, once whole-unit minds have flown for a while. **B:** pull T-54 forward
    between M5 and M6 because ASSIST is the mode most pilots will want.

---

## 22. Build order (milestones and gates)

**M1a — the seam, no Rhai.** Extract `arena/motor.rs` with its tests; move the row formatter into
`arena/view.rs`; add aspect, odds, `reads_dead`, order and `t_ms` to `Frame` and the keyframe
(`FRAME_VERSION` 2, bot parser and goldens updated); `Controller::Mind`, `MindState` with the notice
buffer, `DueKind::MindThink` and `MindLand`, `apply_machine_intents` with the Emcon arm,
`IntentSource::Mind`, `GOTO` as a cockpit verb, the hint re-keying; the native T1 stock mind; the
motor audit tests. Gate: every reflex, step and cross-tempo fixture green; the cross-tempo fixture
with two T1 units byte-identical at 5,000, 1,000 and 500 ms; both harness lanes hold their lenses;
T0 vs T1 over 30 pooled matches inside a Wilson interval containing 50 percent; the six divergences
recorded in doc 16 §11.

**M1b — slots and sliders, no Rhai.** `FireCondition` slots with native evaluation at `WeaponReady`
(its no-op arm filled), sensor refresh and landing; `hold_fire` in `resolve_fire`; `Unwarranted`;
the odds floor; `FIRE <weapon> WHEN …`, `WEAPONSTATUS` arming, attribution and held-shot sentences;
the stock card lifted from `ReflexConfig` as per-unit terms with the six presets, refuse-not-clamp,
and an owner-facing `@mind/set` on T1. Gate: a fixture shows a conditioned shot resolving on the
`WeaponReady` millisecond, not a pulse later; a line-mode transcript of `FIRE cannon WHEN odds >= 60
AND aspect = side,rear` firing only on flanks while a typed `FIRE` still fires; a 30-match pooled
measure of `mush-bot` against a conditioned T1 reported with Wilson intervals; the harness lenses
unchanged.

**M2 — the Rhai runtime.** `crates/mush-mind` (engine builder, op-and-fee budget, canonical memory,
seal cache), `crates/tacsim/src/minds` (view types, kernels, intent functions, `RhaiRunner`),
`content/minds/standard.rhai` with its manifest (Line = `ReflexConfig` defaults), `DrawLabel::Mind`.
Gate: sandbox tests (identical budget trip on two runs; no time, eval or keyword-function symbol
resolves; memory canonicalisation; type faults; the cross-platform hash test); Gate A, T2 ≡ T1 on the
30-run corpus, the cross-tempo fixture and whole-match hashes.

**M3 — hardware, seal, record.** The `core` content kind and `ComponentRole::Core` with
`CoreProfile`, `degrade_core` and `destroy_core`, the `MindSeal` on `CustodyUnit`, the pool with
admission at `SIM MIND`/`SIM UNIT` and check-out, the `MindSealed`/`MindFault`/`MindDropped` ledger
kinds, `MatchRecord.minds`, the source store, snapshot rows and resume, the dormant row across
possess and release. Gate: the label-set and verb-set contract diffs reviewed; a resume between N
and N+1 reproduces the hash sequence; an over-subscribed roster refuses by name; three injected
faults drop a unit to the ROM; `destroy_core` drops one at its next landing.

**M4 — the P3 gate and the flip.** `tacsim_harness_arena_minds` (stock CI, stock authored, ceiling
authored) with jittered wakes and the arena-task pool; AI fill flipped to Standard Doctrine on Rhai;
`Controller::Reflex`, `ReflexTick` and the decision half of `reflex.rs` deleted; `NotReflex` →
`NotMachine`. Gate: every lens holds three times running on the blessing host through
`scripts/harness-weekly.sh`; the deterministic block committed to the baseline; if the stock lane
fails, the fast-path decision (T1 in production for Standard) recorded in doc 16 §11 before the flip.

**M5 — the pilot's experience.** `SIM MIND`, `SIM AI MIND`, sealing at `BOARD`, the `MIND` cockpit
verb and block on line mode, TUI and web at 120 columns, profiles and `MIND PROFILE`, the rule
language compiling to slots with English written back, release-past-grace to the sealed mind, the
`Your mind` block of `SIM REPORT`, the two hints, `help minds`. Gate: a line-mode transcript on the
local rig of a pilot sealing a mind, exiting, and the tank flown by it; goldens and the bot parser
updated; fog asserted (the other side's mind never named, before or after).

**M6 — the workshop, the proving ground and the bench.** The `@mind` switches, content-addressed
revisions, `@mind/check` on canned pictures, schema-rendered cards, the web workshop, the TUI card
screen, SSH push and pull if Q-UM-7 says so; the trace ring and `arena.mind`; the proving executor on
its pool with quotas, both-sides seeds, `wilson_95` in the library, the report; the seat-intent tape
under its bound; the bench with closed-loop and open-loop sessions, STEP, BACK, TRACE, WATCH, EVAL,
DIFF, SWAP, POSSESS, ACT AS; `help mindcraft`, `help proving`. Gate: the `mush-bot` pilot (Picture,
Pilot, Doctrine) ported as a Rhai mind seals, flies and checks within Class II; against Standard over
30 pooled muster matches it lands inside the bot's own pooled interval; a bench re-run reproduces a
proving match's hash at every step; re-stepping every think of a kept OOC match reproduces its
intents and hashes; two running proving arenas leave the live p99 unchanged.

**M7 — the Roll and the market.** `crates/mush-roll`: drafts, filing conformance and the rated
bench, the digest, charters through `issue_title`, lines with follow and pin, licences as seat-term
leases settled by E4 contracts with the levy, venue and owner split, auto-renewal, Grace, lapse to
doctrine at the seal; Entitlement listings through tacsim-registered tables; sealed, open and
open-resale terms; transferable licences with the alt refusal; the house vendor catalogue; IC cores
as titled assets with dies, lineage, the importer, `core_integrate@1`, the install op and the
Class III end-user entitlement. Gate: on the local rig one character publishes a sealed mind, a
second character of another account buys it, tunes it, flies it in the sim and resells the licence
with a balanced ledger, an alt-to-alt sale refused, a lapse falling to doctrine with the line shown
only to its principals, and a derivative's settlement paying its parent chain exactly.

**M8 — contraband and procurement (gated on IC combat).** The governed bit, ungoverning refits,
heirloom and grey provenance, renewal attestation, custody inspection verbs, black-box seal lines,
seizure, fine, revocation, Tycho's cover condition, the `minds.unwarranted` canon event and the Red
Dies front; NPC procurement with the diversity cap. Gate: observer-rule leak tests prove an
ungoverned core is learned only through the four evidence channels; the zero-PC test in which an NPC
org re-equips from the Roll on its own budget with no charter over the cap.

**After M8.** The T-54 cut: per-channel K6 leases, a second station on the tank, ASSIST and `MIND
ON|OFF <channel>` with a second hashed rate counter, mercenary `mind.warrant` delegation
(`Delegation.expires_at`). Real multi-second weapons, after which the slot and cadence measures of
M1b are re-run.

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
  re-simulation of older matches; hence the exact pin, the ABI string, and refusal across versions.
- **Floats across platforms** are deterministic in theory for basic IEEE operations on one binary
  but unproven between the Windows blessing host and the Linux build; the CI test is mandatory, and
  Q-UM-9 B is the fallback.
- **Observer leakage through kernels** — odds expose the silhouette (gated on classification);
  `line_to`, the nav pickers and the condition evaluator leak if a later change traces to live poses
  rather than plots. Each kernel has a hold-the-plot-move-the-truth test.
- **Migration blast radius.** `reflex.rs` is 7,042 lines with about 1,800 of tests; `step.rs` and
  the harness counters assume `ReflexTick`; Gate B may show today's tuned machine stronger than its
  observer-pure port and force a retune before the flip.
- **Conditions will out-shoot typed `FIRE`** under the 1 s placeholder weapons, and whole-unit minds
  plus slots may make "arm slots or lose" the meta before real weapons arrive. Intended by P1; the
  measures are re-run when the placeholder goes.
- **Market balance and monoculture.** One dominant mind sold to everyone, or flying most NPC hulls,
  flattens play; the levers are classes, the pool, the diversity cap and content tuning, which may
  not suffice.
- **Alt and listing abuse.** Laundering through sales (the same-account refusal), sealed-source
  leakage through an author's own `trace()` (their choice), fraudulent claims (only system-run
  proving figures appear on a listing), sparring used to probe a sealed mind (results only).
- **Money machinery ahead of demand.** Licences, loadouts and seals are authoritative action-legality
  families; the Roll, levies and procurement arrive before IC combat exists (D-16-3). M7 and M8 are
  ordered last and M8 is gated on IC combat for that reason.
- **Storage.** Source revisions, seat tapes and proving records need retention sweeps tied to the
  twenty-match window, the reference rules of §15 and `tape_max_bytes`.
- **Complexity for new players.** Five rungs, profiles, classes and licences could overwhelm; the
  defaults (no mind on a new slot, Standard on Line for the fill, a three-line first mind) and the
  three-line opening of `help minds` must stay true.
