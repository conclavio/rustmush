# 17b — Unit minds implementation plan

Opened 2026-10-03 for [doc 17](17-unit-minds.md) (decided 2026-10-02, `e30b9491`), the ledger and cut map
of the unit-minds programme, kept as [16b](16b-combat-implementation-plan.md) is kept for doc 16: a design
note per cut before it starts, an as-built row when it lands, the exact commands run, a reviewer that did
not write the cut. Doc 17 §22 is the build order and the authority on each milestone's scope and gate;
this document records how each one was built. The cuts are named as doc 17 names them (M1a, M1b, M2 to
M8, L); sub-cuts are M1a.1, M1a.2 and so on.

---

## 0. Build discipline

16b §0 applies unchanged: the workspace green at every sub-cut (`cargo test --workspace --exclude
mush-bot` and `cargo test -p mush-bot` on its own), the contract gates re-blessed in the commit that
moves them, all-target Clippy, strict rustdoc, one commit and one push per sub-cut, no second lane, no
version ladders, no fixture impersonating a production module, panic policy unchanged, a non-baseline
stanza for every new verb. The programme adds:

1. **No Rhai before M2.** M1a and M1b are native. A production match stays on `Controller::Reflex`
   until M4's flip; `Controller::Mind` units exist before M4 only in fixtures and the twin lanes.
2. **The twin gates** (doc 17 §17): T0 is the shipped reflex, T1 the native stock mind, T2 the Rhai
   stock mind. Gate A is T1 ≡ T2 exactly (hashes, intents, the cross-tempo fixture). Gate B bounds
   T0 against T2 over pooled seeded matches with a Wilson interval. Every divergence between T0 and
   T1 is named in doc 16 §11 and doc 17 §24 in the cut that introduces it.
3. **The parity rule for the view** (D-UM-4): a mind reads exactly what the cockpit keyframe carries;
   every mind field is first a `Frame` and keyframe field with a `FRAME_VERSION` bump, the row
   formatters produce the same sentences before and after any move (goldens and the bot's
   `arena_contract.rs` pin them), and a tier truncates, never extends.
4. **The hash.** A cut that moves `state_hash` re-blesses the cross-tempo fixtures once, in its last
   sub-cut, with the reason in its ledger row. Before M4 the hash of a production match must not move;
   a variant added to a hashed enum that changes an encoding is a declared re-bless, not a silent one.
5. **Performance.** Both harness lanes hold their lenses at every sub-cut; M4's `tacsim_harness_arena_minds`
   is the P3 gate (doc 17 §18) with its one fast-path trigger; a cut that adds per-pulse work reports
   the harness lines verbatim in its gate.
6. **The bot stays external.** `crates/mush-bot/AGENTS.md` holds: the engine's native stock mind, the
   view and the kernels share no code with the bot; the bot's reader and contract test move with
   declared reader changes, and `cargo test -p mush-bot` is in the gate of any sub-cut that changes a
   line-mode sentence or the keyframe.
7. **Where the record goes.** The 17b ledger row (§3) takes the as-built cells; doc 17 §24 takes the
   as-built amendments to the minds spec; doc 16 §11 takes the ones that touch the arena spec (the
   verb table, the frame, the controller, the deliberation layer); doc 17 §22's milestone paragraph
   gains one line, "Landed <date> at <hash>; see 17b section 3, row X."; 16c rows the cut closes are
   marked as 16c's convention has it.

---

## 1. The cut map

Scopes and gates are doc 17 §22's, abridged; the paragraph there is the authority. A cut's row is
revised by its design-note commit if the plan refines it.

| Cut | Scope | Depends on | Size | Acceptance |
|---|---|---|---|---|
| **M1a** | The seam, no Rhai: `arena/motor.rs` extracted with its tests; the row formatter moved into `arena/view.rs`; aspect, odds, `reads_dead`, the standing order and `t_ms` on `Frame` and the keyframe (`FRAME_VERSION` 2); `Controller::Mind`, `MindState` with the hashed notice buffer, `DueKind::MindThink` and `MindLand`, `apply_machine_intents` with the Emcon arm, `IntentSource::Mind`, `GOTO` as a cockpit verb, the hint re-keying; the native T1 stock mind; the motor audit tests | AP | Large | Every reflex, step and cross-tempo fixture green; the cross-tempo fixture with two T1 units byte-identical at 5,000, 1,000 and 500 ms; both harness lanes hold; T0 against T1 over 30 pooled seeded matches inside a Wilson interval containing 50 per cent; the divergences recorded in doc 16 §11 |
| **M1b** | Slots and sliders, no Rhai: `FireCondition` slots evaluated natively at `WeaponReady`, sensor refresh and landing; `hold_fire` enforced in `resolve_fire`; `Unwarranted`; the odds floor; `FIRE <weapon> WHEN …`, `WEAPONSTATUS` arming, attribution and held-shot sentences; the stock card lifted from `ReflexConfig` with the six presets; `@mind/set` on T1 | M1a | Medium | A conditioned shot resolves on the `WeaponReady` millisecond; a line-mode transcript of a flank-only rule firing only on flanks while a typed `FIRE` still fires; 30 pooled matches of `mush-bot` against a conditioned T1 reported with Wilson intervals; the harness lenses unchanged |
| **M2** | The Rhai runtime: `crates/mush-mind` (engine builder, op-and-fee budget, canonical memory, seal cache), `crates/tacsim/src/minds` (view types, kernels, intent functions, `RhaiRunner`), `content/minds/standard.rhai` with its manifest, `DrawLabel::Mind` | M1b | Large | The sandbox tests (identical budget trip on two runs; no time, eval or keyword-function symbol resolves; canonical memory; type faults; the cross-platform hash test); Gate A, T2 ≡ T1 on the 30-run corpus, the cross-tempo fixture and whole-match hashes |
| **M3** | Hardware, seal, record: the `core` content kind and `ComponentRole::Core` with `CoreProfile`, `degrade_core` and `destroy_core`, the sortie seal (at `STARTUP` leaving a base, the sim's `BOARD` its first implementation) on `CustodyUnit`, the pool with admission, the `MindSealed`, `MindFault`, `MindDropped` and `SortieClosed` ledger kinds, `MatchRecord.minds`, the source store, snapshot rows and resume, the dormant row across possess and release | M2 | Large | The label-set and verb-set contract diffs reviewed; a resume between N and N+1 reproduces the hash sequence; an over-subscribed roster refuses by name; three injected faults drop a unit to the ROM; `destroy_core` drops one at its next landing |
| **M4** | The P3 gate and the flip: `tacsim_harness_arena_minds` (stock CI, stock authored, ceiling authored) with jittered wakes and the arena-task pool; AI fill flipped to Standard Doctrine on Rhai; `Controller::Reflex`, `ReflexTick` and the decision half of `reflex.rs` deleted; `NotReflex` to `NotMachine` | M3 | Large | Every lens holds three times running through `scripts/harness-weekly.sh`; the deterministic block committed to the baseline; the fast-path decision, if taken, recorded in doc 16 §11 before the flip |
| **M5** | The pilot's experience: `SIM MIND`, `SIM AI MIND`, sealing at the pod's hangar exit, the `MIND` cockpit verb and block on all three clients at 120 columns, profiles and `MIND PROFILE`, the rule language compiling to slots with English written back, release-past-grace to the sealed mind, the `Your mind` block of `SIM REPORT`, the hints, `help minds` | M4 | Large | A line-mode transcript of a pilot sealing a mind, exiting, and the tank flown by it; goldens and the bot reader updated; fog asserted |
| **M6** | The workshop, the proving ground and the bench: the `@mind` switches, content-addressed revisions, `@mind/check`, schema-rendered cards, the web workshop, the TUI card screen, the trace ring, the proving executor with quotas and `wilson_95`, the seat-intent tape, the bench (STEP, BACK, TRACE, WATCH, EVAL, DIFF, SWAP, POSSESS, ACT AS), `help mindcraft`, `help proving` | M5 | Large | The `mush-bot` pilot ported as a Rhai mind seals, flies and checks within Class II and lands inside the bot's own pooled interval; a bench re-run reproduces a proving match's hash at every step; two running proving arenas leave the live p99 unchanged |
| **M7** | The Roll and the market: `crates/mush-roll` (drafts, filing conformance, the rated bench, the digest, charters, lines, licences as seat-term leases settled by E4 contracts, auto-renewal, Grace, lapse to doctrine at the seal), Entitlement listings, sealed, open and open-resale terms, transferable licences with the alt refusal, the house vendor, IC cores as titled assets, the install op | M6 | Large | One character publishes a sealed mind, a second character of another account buys it IC, sees it in their sim set, flies it and resells the licence with a balanced ledger; an alt-to-alt sale refused; a lapse falling to doctrine shown only to its principals; a derivative's settlement paying its parent chain |
| **M8** | Contraband and procurement, gated on IC sorties: the governed bit, ungoverning refits, heirloom and grey provenance, renewal attestation, custody inspection, black-box seal lines, seizure, fine, revocation, the quirk claim (`minds.claimed`), NPC procurement with the diversity cap | M7, IC sorties | Large | Observer-rule leak tests prove an ungoverned core is learned only through the five evidence channels; the zero-PC test in which an NPC organisation re-equips from the Roll on its own budget |
| **L** | The lance-leader: an NPC lance-leader as a character principal; the `Order` admission's character-issuer check; the deliberation layer issuing orders as that character under their own warrant; the doc 16 W7 amendment | M1a (independent of M1b to M8) | Medium | A squad with no character leader runs on doctrine; an order with no character issuer is refused by name; the deliberation lane's orders carry the leader; doc 16's W7 row, §6 and §11 amended |

After M8: the T-54 cut (per-channel leases, a second seat, ASSIST and `MIND ON|OFF <channel>`), the
first non-combat station, region sortie closing, the Ceres Simulation Center; each its own entry when it
is planned.

---

## 2. Baselines

Rows are added by the cut that first measures a figure the programme asserts (the minds lane's lenses,
the pooled twin figures, ops per µs on the blessing host) and by any cut that moves one, with the reason.

| Run | Cohort or lane | Figures | Blessed at |
|---|---|---|---|

---

## 3. The cut ledger

One row per cut, a single very long line, appended with the cut's design note before its first sub-cut
and filled by its record when it lands. A cell holds no pipe and no newline; "Landed" names each
sub-cut with its hash and what it built; "Evidence" the loop each sub-cut went through, the gate
results, the sweep numbers, the harness lines, and what was not run; "Deviations" every departure
from doc 17 or the plan and the residuals still open.

| Cut | Design note | Landed | Evidence | Deviations |
|---|---|---|---|---|
