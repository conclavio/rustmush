# 17d — Unit minds implementation plan through M6

Draft 0.1, 2026-10-17, initially checked at `1137c373` and rebased onto
`origin/main` at `7435f954` (doc 18 through IC10) before implementation.
This is the next implementation sequence for [doc 17](17-unit-minds.md): finish the
rule language and the behavior deferred from M5, then deliver M6's authoring,
proving and debugging tools. [17b](17b-unit-minds-implementation-plan.md) remains
the programme's build ledger. No implementation in this note is marked landed.

The owner requested this plan through M6 on 2026-10-17 and deferred M7 until a
separate check that the economy code has stabilized. The owner also accepted the
recorded timing misses as nonblocking: the 300-unit target exceeds expected launch
scale, and isolated misses on the development machine or VPS can reflect host
variability. Timing remains regression evidence; deterministic correctness,
bounded execution and separation of proving from live play remain requirements.

## 0. Scope and starting point

M1a, M1b, M2, M3, M4's implementation, M5 and L are on main. M5's gate is
`e4da2d7e`, its record `47bcb620`; L's implementation is `4d99b788` and `915fc415`,
with its sweep at `3ef6e34a`. Hash v2 and the subsequent measured optimizations
are already landed. The old local `codex/cut-l` branch is a different, ancestral
doc 16 cut and supplies no unfinished work here.

The current production provider is `match_/holdings.rs::DoctrineOnly`; the native
fill decision in `match_/seal.rs::PRODUCTION_FILL_ENGINE` stands. Rhai execution,
the sandbox, source storage, seals, profile switching and the worker pool exist.
Author libraries, general card schemas, retained think traces, proving jobs and
bench re-simulation do not. The current `MIND TRACE` refusal is deliberate.

| Phase | Result | Dependency |
|---|---|---|
| M5rules | Complete native station-rule language and its pilot surfaces | Landed M5 |
| M5r | Restore the named doctrine and motor behavior | M5rules; landed hash v2 |
| M6 | Authors can save, fly, test and debug their own minds | Both post-M5 cuts |
| M7 checkpoint | Review stabilized economy seams and prepare M7's design note | M6 accepted; separate review, not automatic implementation |

M7 filing, titles, licences, paid services, royalties, market listings and core
installation are outside this phase. M8 contraband and procurement still require
IC mind sorties. Doc 18's carrier journey and captain's order queue do not supply
that lifecycle. ASSIST, per-channel mind leases, new non-combat stations and the
IC region-sortie implementation remain later work.

## 1. Decisions for implementation

These are the plan's implementation choices; the two owner decisions above are
the only new owner rulings. Existing doc 17 decisions and M5's as-built rulings
continue to apply.

### 1.1 Ownership and durable identity

Introduce the **draft and revision portion of `mush-roll` in M6.1**, moving that
crate's creation forward from M7 to honor doc 17 §19's ownership boundary. Start
with only the dependencies needed for character-owned workflow records and
canonical data. No dependency on `tacsim`, `mush-market` or `mush-script`; no
licence, title, settlement or procurement behavior. Tacsim supplies game-specific
schema, class, station and sandbox validation through explicit interfaces. M7
extends this owner instead of migrating a second draft store out of tacsim.

Drafts and revisions are bounded, strictly decoded whole-document workflow
records. They do not become surrogate authoritative licence records. Saved
revisions are immutable. The distinct identities are:

- Source identity: SHA-256 of the stored source bytes, using the existing
  `tacsim:mindsrc:<sha256>` owner and resume checks.
- Revision identity: `DefinitionDigest::for_canonical_value` under
  `tacsim.mind.v1`, over the normalized source, schema, presets/profiles and rules,
  declared class, and `MIND_ABI`. Specify normalization once; preserve source
  locations after line-ending normalization and never normalize string contents.
- Selection identity: character, mind and an explicit revision pin or follow-latest
  choice. Resolve follow-latest once at check-out; retain the resolved revision
  and every profile in the seal. Edits never change a running seal.

Represent an own draft explicitly in the seal identity; do not fabricate a Registry
charter, certified rating or paid licence to fit today's string fields. Notes,
edit timestamps and mutable library labels do not change executable identity.

Keep a monotone local revision number for readable `@r7` references alongside the
full digest; shortened digest display never becomes a unique storage key. Reject
ambiguous abbreviations. Commit source and revision references through one
world-side operation, publish RAM state consistently with durable writes, and
test reboot between write stages. A rejected save exposes no selectable revision.

### 1.2 One card and rule model

Replace the fixed `StockCard` assumptions at the authored boundary with canonical
schema, values, profiles and compiled rules. Today `ThinkJob.card`, the Rhai card
wrapper, `SealTerms.profiles` and `ProfileSet` all assume stock fields or preset
codes. M6 must change these together; UI controls alone cannot deliver authoring.

Use doc 17's closed widget vocabulary, refuse invalid values rather than clamp,
and keep a typed Standard Doctrine adapter over the same validated values. Stock
presets stay named cards, not hidden behavior modes. The schema declares station
sections; absent stations make their sections inert and explain why. Profile
capacity comes from the sealed mind's effective class. One implementation owns
validation and canonical encoding for line mode, web, TUI, SSH and sealing.

Keep the actual runtime ABI `fn think(view, card)`, with persistent memory bound
as `this`. Doc 17 §15's older three-argument example is not a new API to implement.
Support the author manifest and optional schema function through bounded metadata
evaluation, not unrestricted top-level execution. Keep Rhai pinned and its denied
symbols, operation fees, memory limits and fault behavior intact.

Native rules and think-evaluated rules share a parsed representation and English
renderer. Native rules evaluate on the existing event schedule. Think-only terms
(`hit within`, `hostiles >=`, `order =`) are refused to a seated pilot and compiled
for the authored mind's think path in M6. They do not create a second interpreter
or an unbudgeted decision loop. Rules override sliders, and a scripted mind's
cited fire still passes the native warrant and floor checks at landing.

### 1.3 Command and transport boundaries

Replace the single staff-gated `@mind` closure with one dispatcher. Author forms
operate on the current character's library. The current staff live-unit setter
becomes the explicit `@mind/live` form; migrate its tests and help in that cut.
Author `@mind/set <name>/<profile> key=value` has one meaning. Do not retain two
ambiguous `/set` implementations. `/publish` and commercial fork/licence operations
remain unavailable until M7; copying open Standard or an owned draft is M6 work.

All clients call the same authoring/check service. `mind.check` needs an explicit
bounded request/response path through protocol, gateway/IPC and world dispatch;
the existing closed `Query` enum does not already provide it. Do not send source
through a panel-subscription hack. Export TypeScript bindings and update protocol
inventory tests with each registered request/feed.

For SSH, retain the successfully authenticated method in connection state. Only
verified public-key sessions may use the exact `mind push|pull|check` allow-list;
select a character owned by that authenticated account. Bound input before parsing,
return structured diagnostics and exit status, and refuse all other exec forms.
Password authentication remains available for interactive play but grants no exec
access. No shell, arbitrary path, forwarding or general subsystem is introduced.

### 1.4 Visibility and runtime boundaries

Character ownership governs library access; account identity governs proving
quotas. Every source-bearing path shares one authorization predicate: source
reads, copy/fork, diff, diagnostics, web Try, SSH pull and bench inspection. Keep
the later sealed-licence projection expressible now: card/rules/intents/ops and
permitted author traces may be visible, source and memory may not. In particular,
`WATCH` or `EVAL` must not recover hidden memory or source through errors.

Preserve M5's selection fog and Doctrine substitution against an opposing human.
Sealing an authored draft in the OOC sim consumes no IC seat and posts no IC
transaction. Proving, checking and bench work use bounded executors independent
of live mind workers and never mutate a live arena or IC object. Read-only bench
evaluation has no station callbacks, no random-state mutation and its own small
budget. `ACT AS` may inspect the other side only in a private proving run.

## 2. M5rules native station rules

This is [17c §6.2](17c-m5-design-note.md#62-the-rule-language-cut-right-after-m5s-gate-oq-4),
expanded where the actual implementation requires a runtime or wire change.
`ConditionTable::arm` currently replaces by weapon; `FrameWeapon.armed` and its
readers are singular. Multiple ordered rules require a new representation and
projection, not just additional parser branches.

| Sub-cut | Deliverable | Acceptance |
|---|---|---|
| M5rules.1 | Typed bounded rule actions/predicates, ordered slot storage, channel-specific admission and evaluation | Rule precedence and slot capacity; classified targets only; plot-based predicates; exact event ordering at 5,000/1,000/500 ms |
| M5rules.2 | Pilot grammar, authoritative refusals, English readback, multi-rule frame/keyframe and all readers | Every grammar form through the real cockpit; matching line/UI state; bot parser and fog contracts |
| M5rules.3 | Integration gate, compatibility declarations, help and as-built record | Default doctrine and new-rule fixtures, twin equality, snapshot/resume, full applicable sweep |

Implement `FIRE`, conditional hold-fire and `LOCK nearest|best_odds|weakest|flank|newest`
with at most four predicates and no `OR`. Use `FIRE <weapon|ALL> HOLD WHEN ...` for
the conditional veto; preserve `FIRE <weapon|ALL> OFF` as disarming, and declare the
surface spelling in help and doc 17. Add moving/stopped/hurt, earned target-section
readings, rounds comparisons, own-section percentage, heat and no-lock terms.
Define units, percentage denominators, unknown readings and deterministic candidate
tie-breaks in parser/evaluator tests before enabling the forms.

Enforce order hold-fire, then matching veto, then first matching rule per channel,
then slider fallback. Typed immediate pilot `FIRE` keeps its existing human-trigger
semantics. A `LOCK` rule belongs to Sensors and must carry/check that channel's
authority and epoch; it cannot borrow Fire authorization from the old
`IntentKind::Condition` arm. Share slot capacity explicitly across rule kinds;
`ALL` expansion must reserve capacity atomically or refuse before changing any slot.
No partial arming after a failed multi-weapon command.
Keep authored rule selection distinct from independently cycling weapons. With
no authored override, Standard's cannon and secondary-weapon slots retain their
current behavior; first-match precedence must not silently serialize them. Pin
that case and name any intended change separately.

Give each rule stable slot identity and explicit read/replace/remove operations.
Repeated arming from a mind replaces its designated slot instead of appending
another rule each think; multiple same-weapon slots survive a pulse. Keep existing
unnumbered arming as a documented single-rule shorthand over this same runtime.

`Contact` targets become lawful only with the required classification, checked at
arming and again at evaluation/landing. `BestHostile` ranges over classified
contacts. Missing earned facts make the predicate false; no rule reads target
truth. Preserve Compact-floor capture and the admission-versus-warrant distinction.

Publish every active rule in order, including veto and lock rules, to line mode,
TUI, web and the mind view at permitted resolution. Update
`arena/{conditions,intents,weapons,step,hash,snapshot}.rs`, `match_/cockpit.rs`,
`minds/api.rs`, the SSH renderers, web cockpit model and bot contract readers.
Keep the existing 120-column and contact-pane budgets; use a bounded summary plus
the explicit rule read for longer lists rather than silently hiding active rules.
Before M6, `MIND RULES` distinguishes current pilot armings from sealed profile
rules; it must not imply the pilot has saved a rule into a revision.

**Compatibility correction to 17c:** the old plan assumed default-valued new
fields could preserve every production pin and reserved the next frame/API change
for M5r. The ordered multi-rule projection and channel semantics require their own
reviewed frame/API/ABI changes when those contracts change. Update the stock
manifest/source identity as necessary and declare one re-blessing at this cut's
gate for every affected pin. Do not maintain a parallel legacy slot lane to avoid
that declaration. The expected first snapshot move is 10 to 11; use the next actual
format if upstream has advanced. M5r advances from the resulting versions.

## 3. M5r doctrine and motor behavior

This is [17c §6.1](17c-m5-design-note.md#61-the-residues-cut-after-m5-name-17b-cut-m5r-or-a-16c-row-the-committers-choice),
sequenced after the already-landed hash v2 work and M5rules.

| Sub-cut | Deliverable | Acceptance |
|---|---|---|
| M5r.1 | Public match mode and objective geometry on frame/keyframe and mind view; Hold behavior in native and Rhai Standard | Keyframe parity; Hold fixture contests the spot; no private ownership/timer leakage; Gate A |
| M5r.2 | Goal preservation across PLACE, wreck motor cleanup, firing-cell memory and arrival through motor notices | Regressions for STOP F/G/I/K at three cadences; unrelated unit goals survive a wreck; snapshot/resume across relevant transitions |
| M5r.3 | One integrated source/ABI compatibility gate, restored descriptions and test coverage | Native/Rhai equality, declared hash re-blessing, production and bot gates, source/version pins |

Expose the mode word and objective cell/letter/radius, never `held_by` or the
capture clock. Pass the public objective into native Standard and implement its
equivalent in `standard.rhai`. For staff `PLACE`, retain the requested motor goal
while invalidating the path/search state so the next motor step replans. Remove a
wreck's motor row before formation-goal fan-out can use it. Record the firing cell
against the actual shot, using a bounded shot-origin association or the permitted
own launch cell on its corresponding notice. A single latest firing cell is not
enough when rounds overlap in flight. Preserve miss-reason redaction after contact
loss, and test overlapping weapons with movement between launches and impacts.
Consume the motor's arrival outcome instead of guessing from proximity. Both
runners make the same decisions from the same public facts.

Review the fourteen deleted tests at `3a3c469^`; restore tests for behavior that
returns, adapted to the mind seam. Update Hold help, SUPPORT acknowledgement,
drone description and leader prompt only where the corresponding behavior has
returned. Fix the stale stock-script header/key count with its digest change.

Accepted STOP D designation behavior, STOP J and Q17 are not reopened. M4.5a's
optional order-expiry/motor-release changes remain separate backlog; this plan
does not bundle them into M5r. Optional optimization work likewise does not delay
the authoring programme.

## 4. M6 sub-cuts

Build the data and execution contracts before the client interfaces. Sub-cuts are
individually reviewable commits; each records actual formats, commands and results
in 17b. The paths below name current integration points and proposed new modules,
not files claimed to exist already.

### M6.1 Character drafts and immutable revisions

Create the small `mush-roll` workflow owner described in §1.1 and its production
registration. Add character-scoped draft/revision records, bounded edits, canonical
revision identity, revision notes and pin/follow selection. Persist source through
the existing source owner. Enforce 20 authored minds per character and 32 KiB per
authored source; open Standard remains available independently. Invalid working
text may be edited and checked, but saving requires a valid envelope, compile and
schema/profile/rule validation. Reference retention includes owned named versions,
the last 50 unnamed revisions, kept match records/tapes, proving runs and open
seals; reserve explicit future references for M7 listings. Removing a library name
must not invalidate an existing sealed or retained revision.
Unreadable reference-bearing records are not evidence that their referents may be
deleted; quarantine or repair them before reclaiming referenced source.

Gate: save/reboot/load round-trips; two characters on one account do not share
libraries; concurrent edits refuse a stale base revision; limits and malformed
records fail by name; source corruption is not silently repaired; prune/reboot
keeps every referenced revision and never makes a sealed source disappear.

### M6.2 Schema cards and sealed authored execution

Implement the canonical model and stock adapter from §1.2. Include custom schemas,
presets, profile add/delete/rename, native rule bindings and think-only rule
execution. Complete the published stock helper surface (`stock::think`, `rules`,
`fire_discipline`, `best_hostile`, `hold_band`, `steer_to`, `hunt`) with declared
fees, bounded calls and no privileged state. The beginner wrapper around Standard
must execute within the class it declares; do not make the full stock routine a
free kernel that bypasses the operation ceiling.

The baseline's full `standard.rhai` is 42,922 bytes, above the authored-source cap.
`/new FROM Standard` therefore creates a compact commented starter that invokes
the published stock helpers; the full shipped source remains a read-only reference.
Verify this starter within the ordinary source/function/op limits. Preserve the
stock getter adapter and use a generic read-only accessor for custom card fields,
so each revision does not need a separately registered Rhai engine or type.

Extend `HeldMind`, `SealTerms`, `MindSeal`, `SealRef`, runtime cards and Rhai card
getters together. Replace `DoctrineOnly` with Doctrine plus the character's saved
drafts. Draft class declarations are bounded execution claims, not M7 filing or
certification. Check declared/effective class, channels, profiles, rules and pool
at selection and again at check-out. Install only the resolved saved source.
Each sealed profile carries or immutably references its complete canonical values
and rule program; today's name-and-digest/preset-code reconstruction cannot supply
an authored profile. A switch atomically replaces card and rule bindings, clears
obsolete native armings and rejects any carried think answer tied to the previous
profile generation. Test this boundary alongside the existing seat epochs.

Separate the current global `fill_class` cap from a chosen pilot mind's class:
the plan adds an operator `max_class` (default III), retains `fill_class` (default I)
as the default AI-fill cap, and adds the lead-only `SIM AI CLASS <0|I|II|III>`
choice for that side's fill. Preserve the existing over-class refusal first: a
declared/rated class above a governed core at I or higher is refused; a missing
core or Class 0 retains the ROM exception. For admitted minds, effective execution
takes the minimum of the declared class, pod core and operator cap; fill additionally
takes the side's chosen cap. Standard's own rating remains I and the default fight stays
unchanged. Preview the actual sealed class and re-admit all affected hulls, including
seated ones, against the same pool at each class/roster/mind change and check-out.
Carry the selected caps across restart and clear the lead-owned choice on lead
change. The Class II gate must run through this production path with a real Class
II core, rather than silently running at I or using a fixture-only class override.

Gate: an authored mind survives save, restart, selection, seal, release and resume;
editing during flight changes nothing aboard; profile switching keeps its ten-second
rule and dormant memory; sealed native and think rules honor precedence; armed and
unarmed fixtures; default Standard adapter parity; both-human Doctrine substitution;
no IC posting or seat consumption. Pin every new hashed field and frame getter.

### M6.3 Author commands and canned checks

Implement the dispatcher from §1.3: new/list/info/source, paste/line/insert/delete,
check/save, card/set/preset/profile/rule, local copy/fork and diff. A paste is a
bounded draft edit terminated by a lone period, with cancellation/disconnect
discarding its unfinished buffer; it cannot consume the ordinary command stream
after the session ends. `/save` freezes a validated revision and triggers the
second author hint once per account. No source edit touches a running mind.

`/check` compiles and runs the three specified pictures (empty, one hostile at
450 m, hurt with two hostiles), each armed and unarmed. Return line/column,
ops/fees, intents, faults, class violations and station refusals; fixtures are
content scenarios reusable by the proving executor. Apply independent compile/check
queue and operation limits so hostile author input cannot block the world pulse.
Draft checks, metadata extraction and WATCH/EVAL use uncached compilation or a
bounded evictable authoring cache. Do not put each edited source into the current
permanent process AST map. Bound the cache for saved revisions too; active runners
retain their compiled `Arc` when a cache entry is evicted. A many-unique-checks
test asserts bounded retained cache state and unchanged live execution.

Gate: full command transcripts, ownership and stale-edit refusals, compilation and
runtime errors through each picture, consistent diagnostics, literal/control-character
handling, no source or memory disclosure through another character's error path.

### M6.4 Retained think traces and the mind feed

Carry structured diagnostic output from collection through landing, joining it by
unit and think sequence to the actual landed/refused outcomes. Keep the last 64
thinks per unit outside `state_hash`; include ops, fees, bounded kernel-call detail,
rule truth values, author trace lines and faults. Preserve the sandbox's existing
8 lines of 120 characters per think. Bound every diagnostic collection, including
repeated kernel calls; truncation is explicit and never changes execution.

Implement `MIND TRACE [n]` (default 3, bounded by the ring), the last-decision/rule
projection, and `arena.mind` at most once per sim second. World-side delivery
authorizes the sealed principal, the setter of their own fill and staff, including
disconnect/reconnect and ownership changes. Apply the sealed-source redaction
policy even while production only holds open Doctrine and own drafts.

Gate: tracing on/off produces identical intents, memory and hashes; refusal,
epoch loss, fault and ROM drop are attributed to the correct think; ring overflow
is bounded; observer tests exercise line, SSH/TUI, web and feed subscriptions.

### M6.5 Bounded input recording and deterministic re-simulation

Add a versioned `tacsim:tape:<match>` record with the opening snapshot, sealed
revision/source references, captured definition/config identity and the ordered
external inputs needed to reproduce the match. Record inputs at the arena's
admission/application boundary, including timing and same-millisecond ordering;
specifically after `PrePulse` coalescing and before application, not before the
last-typed-wins buffer. Raw command text is insufficient. Cover seat intents, controller/epoch changes,
profile/rule changes, accepted leader orders, staff placement/control and stop
events, plus any other nondeterministic input found by a boundary inventory.

Record the chosen step/tempo schedule where replay requires it. Re-simulation
reruns minds and compares their outputs; it must not replay recorded mind outputs
and call that determinism. Compare every substep hash and every think's intents
and memory. Freeze the actual arena definition and config, not only a digest
resolved against current content, and restore an explicit replay timebase rather
than the snapshot decoder's fresh wall anchor. Reuse recorded accepted leader
orders; replay never asks an LLM for them again. Keep the existing objective-picture `SIM REPLAY` separate from this
new bench substrate.

Persist ordered tape chunks and completion/checkpoint cursors through the arena's
world-side persistence path. Do not synchronously write storage in the live pulse.
A resumed snapshot must match the durable input cursor; a gap marks the recording
incomplete. Retain full seals with the tape, since result summaries alone do not
contain every original mind seal.

Tie eviction to the centre's `HISTORY_DEPTH` of 20 and reference accounting in
M6.1. Proposed initial `tape_max_bytes`: 16 MiB per match, configurable with a
bounded validated range, confirmed against a full-duration recording fixture
before this cut lands. On overflow, keep live play correct and mark that match's
bench recording unavailable/incomplete in words; never accept a truncated tape as
replayable. This makes doc 17's "every kept match" promise conditional on a complete
recording: demonstrate sufficient capacity at supported duration/admission bounds
and expose any exception. Record this clarification in doc 17 when the cut lands.
Source/ABI/format mismatch, missing chunks and tampering refuse replay
before any simulation. Reboot must restore completion markers and retention pins.

Gate: replay a kept match containing human commands, release/reclaim, an order,
profile change, rule change and a staff intervention; all hashes/thinks match;
repeat at three cadences and across snapshot/resume. Assert bounded storage and
explicit overflow/eviction/mismatch outcomes. No per-unit picture archive.

### M6.6 Proving executor and reports

Add owned proving-run records that pin scenario, definition, revisions/profiles,
class/config, seeds and engine identity. Execute DUEL/MUSTER/HOLD content scenarios,
1v1 through 6v6, with even N from 10 through 100, each seed flown from both sides.
Opponents are Doctrine, the character's own holdings or mirror. No published-mind
sparring service or IC transaction is introduced.

Use a separate bounded executor (`proving_workers` default 1), one active run per
account and a real-day quota of 2,000 simulated unit-minutes per account. Count all
units and simulated duration; use UTC day boundaries, persist usage, reserve a
bounded allowance at admission and reconcile actual work on completion/cancel.
Restart and retry must not reset usage, duplicate a completed result or charge
twice. Specify a bounded queued-job count and fair admission; disconnect does not
cancel durable ownership or permit a second simultaneous run.

Each worker owns its private arena and drives `Arena::step_settled` with a fixed
synthetic cadence, outside the world task and without `MindPool::shared`. Freeze
the expanded initial state/config; disable external deliberation in proving, or
use explicitly frozen scripted orders in a scenario. Promote production helpers
instead of importing `tests/support`. Bench bulk stepping/forking uses the same
compute allowance so it cannot bypass proving quotas; quick checks have their own
bounded queue and rate limit.

Use `@mind/prove ...` to submit, `/proves` to list owned runs, `/proving <run>`
for status, `/report <run>` for the completed report and `/cancel <run>` to cancel.
Proposed bounded defaults: one admitted queued-or-running bulk job per account,
32 globally queued jobs, and 20 completed runs or 64 MiB per character, whichever
limit is reached first. Evict the oldest unpinned completed run and release its
revision/source references; an active bench pins its run, and admission refuses
when pins prevent staying within the cap. Provide `/forget <run>` with the same
reference checks. Capture limits in config and add reboot/eviction tests.

Put the game's `wilson_95` implementation outside `arena/`, with explicit wins,
draws and timeout denominators. The bot keeps its external runtime and compares
the shared published math through test fixtures, not a new server dependency.
Report wins/losses/draws/timeouts, paired seeds, section losses, weapon/odds and
facet outcomes, rounds, rules fired/held, ops p50/p99 and faults. Say a difference
is called only when the specified intervals do not overlap.

Gate: swapped-side determinism, complete result/report round-trip, quota fairness
and crash recovery, authorization, cancellation and bounded queue behavior. Run
two proving arenas alongside live play and record paired latency measurements;
worker separation, bounded work and continued live progress are hard gates.
Investigate repeatable regression; isolated host timing excursions do not block.

### M6.7 Bench replay and controlled experiments

Add the private bench session and `BENCH` dispatcher. First ship closed-loop
`LOAD <run>/<match>`, STEP/RUN TO/BACK and PIC/TRACE, re-simulating from the pinned
start and checking every reference hash. Then ship `LOAD MATCH <match> <unit>`
over M6.5's recording: reconstruct the historical picture and answer what the
selected mind decides, without claiming a counterfactual match outcome.

Add read-only WATCH/EVAL with source/memory permission checks and a small separate
budget; DIFF over held minds on identical pictures; SWAP as an explicitly labelled
fork; POSSESS/RELEASE and station commands under the manual clock; ACT AS for units
of a private proving run. Forked timelines get distinct identities and never
overwrite the reference run or claim its hashes still match. SWAP validates the
candidate class/stations, installs its card/rules, resets candidate memory and
invalidates the former mind's pending answer before the next think. Open-loop
DIFF candidates keep independent memory over the fixed historical pictures and
never alter the replayed world. Historical inspection remains scoped to authorized
units even when the engine internally reconstructs the whole match. No live seat/lease
or IC custody changes. Bound session count, retained fork state and evaluation work;
disconnect cleanup does not delete the underlying proving record.
Proposed session limits are one bench per account, one reference plus one fork per
session, and 30 minutes idle expiry. WATCH/EVAL uses at most 500 ops including fees,
with bounded result length and no live-engine `eval` symbol enabled. Bulk stepping
yields at substep boundaries to the executor's fair queue and charges the common
unit-minute allowance; cached picture/report reads are free. Make these defaults
configurable within validated bounds and test both exhaustion and recovery.

Gate: forward/back reproduces reference hashes; kept OOC match re-stepping matches
every think; DIFF uses identical inputs; a swap diverges only on its labelled fork;
possession invalidates pending intents by the normal epoch rule; unauthorized
opponent/source access and mutating EVAL fail closed.

### M6.8 Web workshop and TUI cards

Add the explicit bounded protocol requests from §1.3 and implement the web library,
CodeMirror editor with Rhai highlighting, inline diagnostics, schema cards,
rule English, canned-picture Try, Prove and Bench controls. The TUI edits cards
and rules and reads source; it is not a second source editor. Both use the same
saved revision and stale-edit checks as the line commands.

Finish the M5 carries: actual authored mind names in their authorized frame,
slider bars and rule glyphs, multiple-profile rows, long library names and 120-column
wrapping, higher-class fill/odds displays, ROM fallback wording and doctrine-only
tracing/comments that become false once another mind can fly. Preserve the contact
pane budget and explicit Enter-to-send behavior for cockpit prefill.

Gate: editor/check/save/reload/fly journey, shared validation errors, narrow/wide
TUI goldens and web tests, long names and maximum profiles/rules, source redaction,
selection/trace fog, request/feed registration and exported types. No frontend-only
saved state and no source in public match records or feeds.

### M6.9 SSH push pull and check

Implement §1.3's authenticated narrow transport against the same service. Bind the
request to account, selected owned character, mind and expected base revision;
push changes the draft and follows the same explicit save semantics as the other
editors. Pull identifies the requested revision. Check returns the same structured
diagnostics without saving. Test chunked input, byte limits, cancellation and
malformed framing without ever treating source text as a shell command.

Gate: real SSH public-key push/check/pull round-trip, password-auth exec refusal,
cross-character/account refusal, stale write, oversized source and unknown command
refusal; ordinary interactive login and cockpit remain intact. Update design 02's
exec statement to the already-approved narrow exception.

### M6.10 Acceptance and documentation

Port the external bot's Picture/Pilot/Doctrine behavior into a separate Rhai mind
as the authoring acceptance. The bot remains an independent executable using the
public SSH interface; no shared planning implementation or game runtime dependency
is added. Use the public picture and kernels, including fog, throughout the port.

Class II's stock ceiling is **3,000 ops per think at 500 ms**, not its 6,000 ops
per second pool charge. Assert the actual class limits, including source size,
memory, contact/odds rows, slots, profiles, functions and `line_to`. Against Standard,
fly 30 pooled MUSTER matches with paired sides and compare the result with the
external bot's freshly measured pooled interval on the same scenario revision.
Keep wins/draws/timeouts and sample counts visible; do not use a stale historical
interval after M5r changes Standard.

Run the complete author story on the production composition: new, edit, check,
save, select, seal, release, trace, report, prove, bench, edit again and show the
earlier seal remains unchanged. Include an unarmed hull, a faulting mind falling
to ROM, two characters sharing an account, an unrelated observer and an opposing
human. Finish `help mindcraft`, `help proving`, pilot help and the first-save hint.
Record the sub-cut ledger and the exact compatibility/deploy effects. Then stop
at the M7 checkpoint in §7.

## 5. Compatibility and release discipline

| Boundary | Required treatment |
|---|---|
| M5rules | Ordered rules and channel admission; next snapshot format, frame/API/ABI versions where changed, stock identity and hash pins reviewed once at gate |
| M5r | Public objective view and changed Standard behavior; next frame/API/ABI and stock source digest, restored parity fixtures and declared production re-blessing |
| M6.1 | Versioned draft/revision documents; strict decode, immutable source keys and retention; no fake licences |
| M6.2 | General card/profile/rule seal and runtime shapes; snapshot/hash/ABI changes in this cut, stock adapter and resume refusals verified |
| M6.4 | Trace/feed wire additions; diagnostics stay outside simulation hash and do not enter durable live snapshots merely for UI convenience |
| M6.5 | Independent replay/tape format and completion marker; no replay across unsupported engine or content versions |
| M6.8–9 | Request/feed/IPC and generated client bindings; coordinated gateway/server/web release |

Resolve actual version numbers against freshly fetched main before each sub-cut;
do not reuse the historical 3-to-4 or 10-to-11 numbers blindly. Each implementation
names exactly which bytes change and why, updates every consumer in the same cut,
and never blesses a failed correctness test. Snapshot/ABI changes keep the existing
named OOC restart refusal; schedule deploys with no live matches where practical.
No region-sortie migration policy is invented here. Document server/web/bot release
compatibility when a line or keyframe reader changes.

## 6. Verification and ledger requirements

Keep 17b §0's independent review and full sub-cut gates. Performance interpretation
is amended by the owner direction at the top of this note: preserve timing output
and deterministic baseline comparisons, but do not demand three consecutive timing
passes or change thresholds just to hide a noisy run. Functional bounds, op ceilings,
fault accounting, no-IC-crossing and worker isolation are still asserted.

For each cut run its focused tests while building, then the repository's full
applicable gate once for the final tree:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --exclude mush-bot --no-fail-fast
cargo test -p mush-bot
cargo doc --locked --workspace --no-deps --document-private-items
```

Set `RUSTDOCFLAGS=-D warnings` for rustdoc. For reader/frontend changes run `npm ci`,
`npm run check`, `npm test`, `npm run build` in `web`, plus the TUI goldens and bot arena contract.
Run protocol generation when types move. Keep the three explicitly named CI gates:

```text
cargo test -p tacsim --test minds_rhai m2_the_twin_corpus_hashes_the_same_on_every_host -- --ignored --exact --nocapture
cargo test -p tacsim --test twin m2_whole_match_hashes_are_equal_between_the_runners_over_the_pool -- --ignored --exact --nocapture
cargo test -p tacsim --test minds_pool m4_real_tasks_over_the_twin_s_thirty_hash_as_the_library -- --ignored --exact --nocapture
```

Run the existing M5 story/fog and L suites when their surfaces change. Check
`cargo tree -p mush-bot --edges normal,build --target all` whenever the port or
statistics work touches bot dependencies. Add new test targets to CI's inventory
where required. Replay and cross-platform pins run on both supported hosts.

Proposed new suites cover rules, workflow storage, author commands/checks, trace
privacy, input recording, proving, bench and SSH transport. Add them with the cut
that owns the behavior; tests must exercise observable invariants and failure
paths, not merely restate structs. Each sub-cut records scope, commit, commands,
results, format/source/hash changes, deployment effects and remaining carries in
17b. An independent reviewer checks scope and code before the next dependent cut.

### Remaining choices to settle within their owning cuts

- **M5rules.1:** specify ordered author-rule selection and capacity/ALL expansion
  separately from independent ready-weapon slots. Preserve the no-author-rule
  multiweapon behavior; the inherited `slot_parallel` question is not silently
  answered by serializing every fire rule on one channel.
- **M6.2:** retain `MindSeal.card`'s canonical byte-array representation unless its
  schema change requires a different wire encoding; settle 17c D16 explicitly,
  rather than changing JSON shape incidentally. For D17, keep the stable drop cause
  on the ledger and bounded diagnostic detail in the trace; distinguish fault,
  hardware loss and fallback in tests and displayed words.
- **M6.5–7:** measure the proposed tape cap, set bounded proving queue/session and
  read-only-evaluation limits from the proposed defaults above, and pin restart/overflow behavior before enabling
  the public verbs. These are operational defaults, not economy-policy questions.

None requires reopening the settled character holdings, opposing-human substitution,
pilot slot grammar or public-key SSH decisions. M7's open-fork and pending-transfer
defaults remain assigned to M7's later design review.

## 7. Trade-run coordination and the M7 checkpoint

M5rules, M5r and most M6 code can proceed alongside doc 18. Coordinate changes to
`tacsim/src/lib.rs`, `composition/mod.rs`, config, command/contract inventories,
content lists and shared help. Keep registration edits small, fetch before landing,
and re-run composition tests against the then-current trade tree. Do not change
port commerce, travel, lease or settlement behavior to support M6's draft workflow.

**IC10 reconciled at `7435f954`:** the plan branch is based on its landed market
code. It adds `ProceedsPolicy`/`SaleTerms`, wired to tacsim's `CoopProceeds`, and
`transfer_marked`/`inventory.transferred` for commodity delivery. The proceeds
interface currently handles sales into standing commodity bids, with one extra
recipient (`MAX_PROCEEDS_CUTS = 1`) and one installed policy. It is not yet the
general entitlement levy/venue/parent-royalty split M7 needs, and an inventory
transfer marker is not a licence-issuance receipt. M7 must design the appropriate
entitlement adapter or a deliberate bounded extension while preserving Co-op and
lot behavior. IC10 changes no arena, mind, workshop, TUI or SSH implementation.

The [IC10 cut note](18-cut-notes.md) also carries checkpoint inputs: boot-healed
settlement counters, the hold-backed player path without a Fact gate, deliver
submissions dropped under queue saturation, and `inventory_applied` retention
limited to 4,096 markers. The note requires the retention fix before a deployment
that enables selling. Track these under the trade/economy work and verify their
dispositions at the checkpoint; they are not additional M6 implementation scope.
Preserve IC10's broader `Ctx::authoritative_pending` behavior when adding M6 request
handlers to the shared command infrastructure.

After M6 acceptance, review the actual merged economy tree before starting M7.
IC10's arrival supplies the first important sale/proceeds synchronization point;
it does not itself complete the stability review. Completion of every IC11–IC13
feature is not itself the readiness criterion. Check:

1. Catalogue, availability and delivery provider interfaces, party/venue/capacity
   checks and proceeds-policy composition are stable enough for an Entitlement
   provider; lot-sale changes do not accidentally dictate entitlement semantics.
2. E4 escrow, settlement splits, refunds, idempotence, expiry/renewal and outstanding
   lease behavior have passing integration and reboot/replay tests on merged main.
   Confirm the account and title/issuer capability interfaces M7 will consume.
3. No in-flight trade change is about to rewrite the specific interfaces M7 would
   bind. Coordinate shared files and rebase the M7 note on those commits, including
   any remaining IC13 lease/buyout changes that affect those semantics.
4. The M6 draft/revision owner, holdings seam and source references can be extended
   with authoritative typed licences without moving ownership or creating another
   registry. Resolve Q-UM-17 and Q-UM-18's written defaults in that design note.

Record that checkpoint in 17b with the checked commit and test evidence. M6 finishing
does not automatically launch M7. No calendar delay or arbitrary quiet period is a
substitute for this interface and correctness review.
