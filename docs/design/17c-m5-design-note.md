# 17b cut M5 — the design note (REVISED after the feasibility and adversarial reviews and RECONCILED against both read whole; committed to 17b §1 and §3)

Planned read-only at `5f505c8` (branch `claude/amazing-allen-q1198y`, M4's record commit: M4 landed but
for the owner's three weekly runs on the blessing host, 17b cut M4.7), and revised at the same commit
after two reviews (a feasibility lens and an adversarial-completeness lens; their dispositions are §9).
The first revision received the adversarial review cut off in its sixth finding; on 2026-10-11 every
finding of both reviews was re-read whole against this note and the ones not yet applied were applied
(§9 lists all 47 with where each landed; `briefs/M5-review-reconciliation.md` is the check).
Binding: the owner's rulings of 2026-10-10 (`briefs/owner-rulings-M5.md`, OQ-1 to OQ-12), which adopt
the programme lead's note for M5 as proposed. Inputs: `briefs/M5-map-critic.md` (the synthesis; its §5
scope and carries, §6 split, §7 decisions, §9 unverified) and the four maps it names
(`M5-map-design.md` R-1…R-62 and C-1…C-10, `M5-map-code.md`, `M5-map-carries.md`, `M5-map-tests.md`).
The model is 17b's M4 rows (cut map 17b:59, ledger 17b:115). "doc 17" = `docs/design/17-unit-minds.md`,
"17b" = `docs/design/17b-unit-minds-implementation-plan.md`, "doc 16" = `docs/design/16-combat-arena.md`;
a bare `path:line` is under `crates/tacsim/src/` unless it names a crate, `tests/` is
`crates/tacsim/tests/`. Every fact was read at `5f505c8` by this note, the critic's 32 spot-checks or
the two reviews; what stays unverified is listed in §10 with the sub-cut that settles it. Line numbers
are `5f505c8`'s: **find every site by name**.

---

## The note's head (the 17b ledger row's opening, as committed)

2026-10-10 (doc 17 §22's M5, the seventh cut of the unit-minds programme; planned read-only from
`5f505c8`, M4's record commit, and revised the same day at the same commit after the feasibility and
adversarial reviews, reconciled 2026-10-11 against both read whole, on the programme lead's note for M5 as the owner adopted it on 2026-10-10 (OQ-1),
which outranks doc 17's prose where they differ and never its decisions without saying so: no
licence, no market, no library or workshop (M6); no Rhai on a production path (`PRODUCTION_FILL_ENGINE`
stays `Native`, `match_/seal.rs:160`); rhai pinned; the bot external and never typing `SIM MIND` or
`MIND`; no `STANDARD_SHA256`, `MIND_ABI` or `FRAME_VERSION` move and no production hash pin moved; ONE
`SNAPSHOT_FORMAT` move; no deploy until M5's gate, then one deploy and one void; `git fetch` before each
commit, never force; seven sub-cuts, the critic's eight less the rule language, which is its own cut
right after M5's gate (OQ-4), with the doctrine and motor residues a separate cut after M5 (OQ-6)).
**The pilot's experience on the one mind that exists: a holdings seam that holds Standard Doctrine and
its six presets for every character (a test provider holds two scripted fixture minds); `SIM MIND` and
`SIM AI MIND` choosing a mind and up to the sealed class's profiles, fitted at the choice (class,
over-class, floor on an armed hull, profile count, the pool with seated hulls counted) and fogged to
the slot's own character and the lead; the choice carried into the seal at check-out with every
default seal byte-identical, the sealed profile set on `SealRef` and the active profile on
`UnitState.card_profile`, in the one format move that also carries STOP E's charge and walks the motor
row's inner shapes; the released hull flying its sealed profile with the `Withdraw` seed gated on the
doctrine bit; the board, the reconnect, the release acknowledgement and the catch-up saying "its mind";
the seventeenth cockpit verb `MIND` with `MIND PROFILE` as a seat-only intent, its block on line mode
and as the weapons pane's tail on the TUI and the web at 120 columns with the think lines "dormant
while you fly"; `SIM REPORT`'s `Your mind` block; the first hint; `help minds`; and the help, the
`SUPPORT` acknowledgement, the drone note and the leader's prompt made true of what the machines do
now.**

---

## 0. The findings at head that shape the split (each verified at `5f505c8`)

- **F1. One mind exists and nothing holds it.** Standard Doctrine is identity constants
  (`SEALED_MIND`/`CHARTER`/`LINE`/`LICENCE`, `arena/doctrine.rs:351-363`); no library, drafts,
  licences or `mush-roll` crate exist; the only mind-keyed family is the source store
  `tacsim:mindsrc:<sha256>` written once at boot (`store.rs:97-110`, `lib.rs:509-518`) through
  `store::put_mind_source(ctx, …)` (`store.rs:905`, `pub(crate)`, needs a `Ctx`; the RAM map
  `TacsimStore.mind_sources` is `pub(crate)`, `store.rs:235`). So D-UM-49's "IC holdings and Doctrine"
  is {Standard Doctrine, six presets} for every character in production, every other path is fixture
  reach (OQ-2), and an integration rig cannot store a fixture's source without a seam that does it.
- **F2. The seal is already taken at the hangar exit.** Every hull is sealed at `Boarding → Running`
  in `match_::start` on a fixed Line card (`match_/mod.rs:1103-1133`), and the last `BOARD` runs that
  check-out inside its own command (`match_/cockpit.rs:2118` → `match_/lobby.rs:2431`), or the boarding
  deadline's coarse tick does (`lobby.rs:2431-2489`) — so `ctx.actor` in `run` is not each pilot. 17b
  cut M3.3's "the seal at `BOARD` is the seal at check-out" (doc 17:1793). What M5 adds is the choice
  feeding the seal and the one line before `STARTUP`.
- **F3. The sealed card never reaches the mind that flies.** The arena keeps `SealRef`
  (`arena/state.rs:2110`, written `:4550`), which carries no card, and `adopt_mind` lays
  `MindState::fresh`'s Line (`arena/mind.rs:1453-1467`, `:816-833`), for a released seat and — through
  `schedule_first_mind_thinks` (`:1417-1426`) — for the fill. Today both are Line, so nothing
  diverges; a chosen profile must cross the seam as state the arena carries.
- **F4. A skipped-when-empty seal field moves no digest.** `MindSeal::digest` is SHA-256 over the
  record's own serde JSON (`arena/mind.rs:518-523`; the snapshot is JSON, `arena/snapshot.rs:571-575`):
  a `#[serde(default, skip_serializing_if = …)]` field leaves every default seal's bytes — its digest,
  item 3c's `seal_digest` fold (`arena/hash.rs:960-963`) and every `mind_sealed` line — unchanged, and
  still decodes. `SealRef` is never folded (`arena/hash.rs:218-223`; the probe `:2300-2323` holds each
  of its fields out of the hash), a per-match constant. `SealRef::doctrine`'s doc ("a `MindSeal` field
  would move every seal digest", `arena/mind.rs:384-386`) is true only of an always-written field.
- **F5. The production pin fields a seated hull, and `UnitState.profile` is taken.**
  `m1a5_the_production_hash_does_not_move` builds `BLUE` as `Controller::Pilot`
  (`tests/minds.rs:3182-3188`); a dormant row laid for every seated hull at check-out would add a row
  to item 3c and move it. The active profile therefore lives on the unit, and the row is still born at
  the release. `UnitState` already has `profile: UnitProfile` (the hull's silhouette and what a hit may
  strike, `arena/state.rs:2079-2080`), and "profile" already names `Arena::mind_profile` (the class
  row's `CoreProfile`, `arena/mind.rs:1478`) — so the new field is **`UnitState.card_profile:
  ActiveProfile`**, and `snapshot.rs`' `every_unit_field` (`:1245-1300`, 43 names) decides it carried.
- **F6. No intent switches a profile, and none needs a mind's station.** `IntentKind` has no such
  variant (`arena/intents.rs:121-260`). D-UM-10 closes the verb set one way for minds (a mind has no
  verb a pilot cannot type) and asks each added intent for its STATION verb (doc 17:1384): a seat-only
  intent with the cockpit's `MIND PROFILE` registers no Rhai function, so `MIND_ABI` and every seal
  digest stand (`minds/mod.rs:154-166`). Its refusal word stays outside the mind's API too, so
  `API_VERSION`'s "any … refusal" rule (`minds/mod.rs:154-156`) is not engaged: a mind reads refusal
  words only through the `Notice.refusal` getter (`minds/api.rs:455`, `:916-920`), a seat's notices
  never enter a mind's buffer (`stage_notice` pushes to no dormant row, `arena/step.rs:3440-3459`), and
  a mind-sourced `Profile` is refused `channel` before it lands (no station function builds one).
- **F7. A field for the pilot is not a frame version, but it must be a frame CHANGE.** `FRAME_VERSION`
  bumps only for a field added for a reader that is not a person (`match_/cockpit.rs:5662-5678`); a
  `mind` object built for the seated pilot and never put on `MindView` is a tier truncation (D-UM-4,
  17b §0.3). `Frame` has no `Serialize` and `last_frames` decodes empty (`snapshot.rs:128`, `:901`):
  never carried, never hashed. The arena sends a frame only when `Frame::same_as` says it changed
  (`arena/step.rs:419-489`, each pilot-read field listed with its reason; `t_ms` never compared), so a
  new frame field a pilot reads joins `same_as`. Every M5 seal is Class I (`STOCK_RATED_CLASS`,
  `arena/mind.rs:141`; `fill_class` I, `config.rs:712-713`), whose row carries two odds rows and two
  profiles (`arena/core.rs` `ClassTable::STOCK`), so the odds-row widening "held for M5's
  `FRAME_VERSION`" (doc 17:1791) loses nothing by waiting (OQ-10 c).
- **F8. Nothing feeds the block's think lines.** The trace ring and `arena.mind` are M6's (doc
  17:1605); the native engine counts no ops (`ThinkResult`, `arena/mind.rs:951-963`); the block's only
  reader is the seated pilot, whose mind is dormant (`Arena::suspend_mind`) (OQ-8).
- **F9. Seated hulls load nothing at the pool.** `pool::admitted_class` admits a seated hull at
  nothing (`match_/pool.rs:78-89`) though it is sealed at `fill_class`, and `pool_at_the_choice` is
  passed `None` for every claimed slot (`match_/lobby.rs:1577-1588`), so `SIM UNIT` cannot refuse
  (`:1590-1605`) — the carried "seated hulls at the pool" (doc 17:1808).
- **F10. The board's release word is a bot's resume key.** A drop prints `, flown by doctrine` and an
  `EXIT` `, left` (`match_/lobby.rs:3906-3914`); the bot reads them as `Standing::{Doctrine, Left}`
  (`crates/mush-bot/src/arena/lines.rs:1040-1044`) and resumes on the first (`docs/bot.md:429`) — 27
  sites in 10 crate files plus three docs (critic G4). The board's `[type]` prints in `Selection`
  only (`match_/lobby.rs:3928-3946`), the shape the mind clause copies; the bot strips suffixes before
  `[type]` (`lines.rs:1031-1072`), so any new suffix is a declared reader change.
- **F11. `SIM AI MIND` collides with `SIM AI`'s number** (`How many a side? \`SIM AI 2\`.`,
  `match_/sim.rs:234-248`); `lead_character_of` answers only in `Boarding` (`match_/lobby.rs:705-716`),
  so the lead's character in `Selection` comes from `Centre.leads` and the lead's own slot; and a side's
  lead is an account that `Centre::resettle_leads` (`lobby.rs:311-334`) moves to the side's next
  claimant at `SIM LEAVE` (`:1109`), at the boarding deadline (`:2380`), at `slot_left` (`:2248`) and on
  a reset (`:2783`).
- **F12. The tape has no hand-over line and no profile names.** The kind words
  (`arena/ledger.rs:666-702`) have no release or possess line; `mind_sealed` carries mind, source,
  card_digest, class, governed, licence and principal (`:569-590`), no profile; `Fired.by` is `cond`
  for a slot's shot and `mind` for a mind's cited shot (`:268-276`; `arena/weapons.rs:107-118`), so a
  pilot's armed slot and a released mind's both fire `cond`. The pilot's report fold is already seeded
  at check-out with facts the tape does not carry (`Fold::with_own_tank`, `match_/mod.rs:1312-1384`,
  `report.rs:575`). The solo rig ends an abandoned match 10 s after its last pilot leaves
  (`crates/mushd/tests/sim_solo.rs:141`, `d3_…` `:766-863`) and loads no help topics (`:167-291`); every
  pilot-facing surface is swept for sixteen hex digits (`sim_solo.rs:740-747`,
  `crates/mushd/tests/observer_rule.rs:2843`, `tests/report.rs:46-63`). A released cold tank starts
  itself up (`tests/reflex.rs:2814`, `t1_a_released_cold_tank_starts_itself_up_and_stays_in_withdraw`),
  so a `startup` line on a pilot's hull after a cold `EXIT` is the mind's alone — the gate's proof (§4).
- **F13. The pool's call-off short form exists.** 17b cut M4.3 gave `StartError::Pool` and the two
  `InstallError`s their call-off words (`StartError::call_off_words`, `match_/mod.rs:891-931`), so
  17b:114's "to be given a callsign and a short form" is discharged but for the long form's hull.
- **F14. Any rule-language field is a carried, hashed shape.** `FireCondition`
  (`arena/conditions.rs:180-195`) is armed by the stock mind in every production match
  (`stock_mind::arming_for`, `arena/card.rs:43-44`), so each slot-fitting term is a `state.conditions`
  shape change — a format move and an `af1` re-pin of its own (critic G8). `MindSeal.armed` is written
  empty and read by nothing (`match_/seal.rs:295`).
- **F15. Every shipped core is governed, and only the heavy's is Class III.** `content/cores/cores.json`:
  doctrine I (scout, drone), line II (every line tank, the bipeds), command III (`heavy_mk1`), all
  governed; `target_mk1` fits no core (`tests/content.rs:1283-1288`) and `seal_for` records a coreless
  hull `governed: true` at `CoreClass::Zero` (`match_/seal.rs:239-252`). The one fixture mind,
  `tests/support/load_iii.rhai`, is rated III and calibrated to 4,300 to 4,499 ops a think (its header;
  digest pinned at `tests/harness.rs:9473-9490`): over-class on every Class I and II core, and on the
  heavy sealed at `min(III, III, fill_class I)` = I, where it faults `ops` every think and drops to the
  ROM. Two in-source tests seal rated III on the governed line core on purpose (`arena/core.rs:645-656`;
  `match_/seal.rs:708-743`).
- **F16. 58 code comment lines promise "17b cut M5"** — **the F16 grep**, `rg -n "cut M5\b|until
  M5|before M5|M5's|to M5\b|M5 admits" crates web/src docs/help` (the draft's `grep` missed `M5's`
  and `M5 admits`, so `arena/conditions.rs:53` and `:146`; `web/src` and `docs/help` hold none at
  `5f505c8`; the old roadmap's unrelated "M5" in the economy, scene and mushd demo files matches none of
  these forms): some M5 makes true, some belong to the rule-language cut, some to the residues cut.
  Each is re-pointed in the sub-cut that edits its file (§3, each sub-cut's "Comments made true"; the
  files no sub-cut edits are M5.7's), and M5.7 runs the grep and records its result in the Evidence
  cell: every remaining hit a true sentence (past tense, or naming the rule-language cut, the residues
  cut, M6 or the cut that rates a mind above Class I).

---

## 1. The scope, placed

Each item names its sub-cut (M5.1 to M5.7, §3) or where it goes instead, with the ruling.

### 1.1 Doc 17 §22's deliverables (`17-unit-minds.md:1587-1594`; authority over 17b:60)

| # | deliverable | placed | ruling, reason |
|---|---|---|---|
| 1 | `SIM MIND` | **M5.1** | OQ-2 (holdings = Doctrine + a test seam), OQ-3 (profiles = presets) |
| 2 | `SIM AI MIND` | **M5.1** | the lead's, from the lead's holdings (Q-UM-13 confirmed by OQ-2); staff Doctrine only (D9) |
| 3 | sealing at the pod's hangar exit | **M5.2** | the seal stays at check-out (F2, C-2); the choice feeds it; the one line before `STARTUP` |
| 4 | the `MIND` verb and block, line mode / TUI and web, at 120 columns | **M5.4** (verb, line mode, the keyframe object) / **M5.5** (TUI, web: the weapons pane's tail) | OQ-8: seal facts, profile, channels, rules; think lines "dormant while you fly"; `MIND TRACE` refused in one sentence; the card's slider drawing and the rule glyphs are M6's (C-22) |
| 5 | profiles and `MIND PROFILE` | **M5.1** (choice), **M5.2** (the arena half: carried set, active profile, intent), **M5.4** (the verb) | OQ-3: one of the six preset names, up to N by the MIND's class (Class I → 2), no slider editing |
| 6 | the rule language compiling to slots with English written back | **not M5** — its own cut right after M5's gate | OQ-4 (§6.2) |
| 7 | release-past-grace to the sealed mind | **M5.2** (flies the sealed profile; the seed gate; Q-UM-15) / **M5.3** (its words) | OQ-5: Doctrine at any preset flies; the substitution built only for non-Doctrine seals |
| 8 | the `Your mind` block of `SIM REPORT` | **M5.6** | OQ-9 (ii)'s content |
| 9 | the two hints | first **M5.6**; second **M6** | OQ-10 (a): it fires on `@mind/save` and names `@mind/prove`, both M6 verbs |
| 10 | `help minds` | **M5.6** | its three-line opening pinned true (doc 17:1740-1742) |

### 1.2 The italic items (`17-unit-minds.md:1595-1601`) and §11/§12's binding sentences

| item | placed | ruling, reason |
|---|---|---|
| list and accept only the character's (or the lead's) IC holdings and Doctrine (D-UM-49) | **M5.1** | OQ-2: the seam; production = Doctrine; keyed by the character who boards (`Slot.pilot`, `match_/lobby.rs:76-79`) |
| a fixture that a mind the character does not hold is refused | **M5.1** | against `DoctrineOnly` (any name but Standard Doctrine) and against the test provider (any name outside its answer) |
| a fixture that the sim seal takes no seat and posts nothing | **M5.1** (the choice) + **M5.2** (the seal) | the write list of a check-out with a chosen mind equals the one without (`board_writes`/`pump_writes`, `tests/match_lifecycle.rs:6206-6227`) |
| the cockpit's sentences speak of the sortie where a match is not meant | **M5.2** (the seal line), **M5.3** (the audit) | ruled C-11a below: a sentence about the hull's mind or seal says *sortie*; one about the seat or the result says *match* |
| the `MIND` block takes the weapons block's place on an unarmed hull | **M5.4** (line mode's channel line) / **M5.5** (the tail replaces `(no weapon)`) | — |
| the sim governor admits the character's own drafts | **M6** | OQ-2: drafts and `@rev` pins wait for M6; vacuous in M5 (no draft exists) |
| Q-UM-15's default (Doctrine against humans) | **M5.2** | OQ-5 and C-7: vacuous in production; the substitution built for non-Doctrine seals only, fixture-reachable; the fill at the lead's chosen preset; taken at the seal (a declared departure, §7 4) |
| D-16-3's row gains D-UM-50's no-IC-crossing clause | **M5.7** (the record) | OQ-2's clause: land it in M5, the boundary is user-visible now |
| R-60: a card section driving a station the hull lacks "is shown inert with the reason … and never refuses the seal" (§11 :864-866; D-UM-43) | **M5.1** (the choice) + **M5.2** (the seal) | C-23: the floor probe runs only on a hull with a weapons station |
| R-61: unarmed hulls run Standard Doctrine with its fire half absent; `SIM MIND` on `drone_mk1` accepts Doctrine | **M5.1** | follows from C-23; the reading M5 relies on is Q-UM-12's default (D-UM-48, doc 17:1469-1472: "the ROM runs Standard Doctrine with its fire half absent"), unruled by the owner and so standing |
| R-24/R-25: the block's six rows at 40 columns; the first three rules' state glyph; the card drawn "one slider a line with a 10-cell bar" | **M5.5** (the rows) / **M6** (the card drawing, the glyphs) | C-22, under OQ-8 (the block is the seal facts, profile, channels and rules); a reading the owner may veto (§7 5) |
| R-9: the pod emulates the chassis's content-declared core and never reads owned IC hardware (D-UM-37) | **M5.1** (item 4) | `core_of_type` reads the unit spec's `ComponentRole::Core` and the store's `CoreDef`, never a character's hardware; nothing IC is read (D-UM-50) |
| R-27: who reads what of a sealed licence | **M7** | vacuous in M5: no licence exists (OQ-1) |
| R-51: the leader's `doing:` histogram and the catch-up read a state word, never memory; a non-Doctrine mind reads `unknown` | **M5.3** (the catch-up, D14) / no code (the histogram) | the histogram already prints the state word (`deliberation/request.rs:586-592`, `unknown=n`); fixture-only, noted with D14 |

### 1.3 The earlier cuts' M5 deferrals (critic §5.1; design map §3.2 — the index omits them, C-10)

| deferral | where recorded | placed | ruling, reason |
|---|---|---|---|
| `BestHostile` ranging over classified contacts | doc 17:1778; 17b:112 | rule-language cut | OQ-4 |
| `Contact` slots, with their own sentence | doc 16:1128; 17b:112 | rule-language cut | OQ-4 |
| more than one slot a weapon | 17b:112 | rule-language cut | OQ-4 |
| the conditional `HOLD FIRE` | doc 17:486-488; 17b:112 | rule-language cut | OQ-4 |
| the eleven `[M2]` sliders | doc 17:1786; `arena/card.rs:41-55` | **M5.6** (said in `help minds`), no code | OQ-10 (e): shipped marked inert, as `read_back` marks them |
| odds rows past 2 | doc 17:1791; `arena/step.rs:314-324` | the cut that rates a mind above Class I | OQ-10 (c); F7 |
| the over-class refusal at the seal | doc 17:1791 | **M5.1** (at the choice) + **M5.2** (the seal's backstop) | OQ-10 (d): built against the fixture mind (Load III), the rule exact (C-24) |
| an operator floor above 100 refuses every Line-carded check-out "until M5 lets a principal choose a card" | doc 17:1793 | **M5.1** | a principal may choose a lawful preset (refuse-not-clamp at the choice); an unchosen ARMED slot and an unchosen fill stay Line, so such a floor still calls a match off at check-out for an armed hull (declared; the default is as built, OQ-9); an unarmed hull is never refused (C-23) |
| the pool wording | doc 17:1795; 17b:114 | **M5.1** | OQ-11: "This tank would think …"; doc 17 §9's example corrected in the record |
| `StartError::Pool`'s call-off short form | 17b:114 | discharged at 17b cut M4.3 (F13); **M5.1** names the hull in the long form | — |
| `MindSeal.card` as a JSON integer array | 17b:114 | stays owed (D16) | profiles are sealed as names and card digests, so the seal does not grow by a card per profile |
| `MindDropped.fault`'s open ruling | doc 17:1794 | stays open; **M5.6** prints the cause word only | D17 |
| "M5 seats one" (the principal alone in the arena) | doc 17:1794 | discharged: a seated hull's principal is its pilot since 17b cut M3.3 (`match_/mod.rs:1111-1119`); **M5.6** carries the drop's cause in the pilot's own block | — |
| the lead's fill class (D-UM-37, Q-UM-3 A, "until M5 lets the lead set a class") | doc 17:1795 | the cut that rates a mind above Class I | OQ-10 (b): only Class 0 or I is meaningful while every mind is rated I |

### 1.4 The carries (doc 17 §24's index, `17-unit-minds.md:1808`, and the carries map's §3)

| carry | placed | ruling, reason |
|---|---|---|
| STOPs A, F, G (with `fan_goal`'s wreck read, `arena/motor.rs:2505-2524`), I, K | **the residues cut** after M5 | OQ-6, OQ-7: the behaviour after M5, one declared re-bless sequenced against 16c T-104 |
| STOP D | **M5.6** (the text) | OQ-6: the help, the `SUPPORT` acknowledgement and the leader's prompt say what is true; the designation half recorded as accepted under D-UM-15 |
| STOP J | record only | an accepted divergence (doc 17:1245-1252: true pose is a leak) |
| STOP E's two residuals | **M5.2** rider | `MindState.order_charge`, folded inside the order's own bytes (the owner's 2026-10-09 note: "moves every Support fixture's hash once") |
| the doctrine bit's ungated `Withdraw` seed | **M5.2** | gated on `SealRef::doctrine`; no production hash moves (every production seal is Doctrine) |
| the help gaps (A, D, the support-end sentence) | **M5.6** | OQ-6: the text in M5, the behaviour in the residues cut, its text restored again there |
| `standard.rhai`'s stale header (and "fourteen keys" at `:4`, `:128`) | **the residues cut** | it moves `STANDARD_SHA256` and through the seal every production pin; it rides the cut that moves the digest anyway |
| D14 (no catch-up phrase for `unknown`) | **M5.3** | one phrase in three renderers, naming no mind |
| `MotorRow`'s inner shapes under no pin | **M5.2** rider | the `af1` pin "moves only together with a `SNAPSHOT_FORMAT` bump" (`tests/arena.rs:5680-5682`) |
| seated hulls at the pool | **M5.1** | F9 |
| a configured class of fewer rows than the seat's two | **M5.1** rider | `validate_classes`' `rows` floor 1 → 2 (`config.rs:923`) |
| M4.5a's consequences ((1) no `order_expired` from a mind's end; (2) the motor row after an `ORDER` release; (3) Q17's stand) | (1), (2) **the residues cut** if the owner wants them; (3) record only | OQ-6 |
| the leader's system prompt over-promises A and D (`deliberation/request.rs:539-548`; "own reflexes" `:541`, `deliberation/squad.rs:61`) | **M5.6** | OQ-6's text |
| the release acknowledgement's "to doctrine … on its own reflexes" (`match_/cockpit.rs:7909-7915`) | **M5.3** | fog: one word for every release |
| "its fourteen keys" at `arena/mind.rs:1170-1171` | **M5.2** (a Rust comment in a file it edits; no digest) | the `.rhai` twins stay with the residues |
| `arena/hash.rs:741-742`'s "an arena with none (every production match) folds no byte here" | **M5.2** (a comment in a file it edits) | stale since M4.3 |
| the 58 "17b cut M5" comment lines (F16) | **M5.1, M5.2, M5.4, M5.6, M5.7** by file | each made true, or re-pointed to the rule-language cut or the residues cut |
| `arena/stock_mind.rs:887-893`'s reflex prose | not M5's | inside the ~170 declared lines (doc 17:1808) |
| doc 17 §9's prose at `:755` (`SIM UNIT` "counts no fill") | **M5.7** (the record) | stale since M4.3 |
| **Not M5's** (the index): the owner's three weekly runs; the ~170 lines of reflex prose and the ops-per-µs corpus; `twin_rhai_timing`'s re-measure; 16c T-104 to T-107 and the VPS spike list; the `slot_parallel` ruling | — | OQ-12: the runs stay the owner's, independent |

---

## 2. The settled questions

### 2.1 Doc 17's contradictions, each ruled

- **C-1 `COCKPIT_VERBS`' count.** D-UM-10 says "grows to 16 with `GOTO` and `MIND`" (doc 17:1384), §4
  and §19 "15 → 17" (:291, :1348); built: 16 with `DRIVE` (`match_/cockpit.rs:205-222`). **Ruled:**
  `MIND` is the seventeenth, as the M1a.4 amendment already says (doc 17:1754); M5's §24 bullet records
  D-UM-10's row as read "17, with `DRIVE` and `MIND`"; the decision row is not edited.
- **C-2 The pilot's seal at `BOARD` or at check-out.** Ruled **at check-out**, as built: the last
  `BOARD` runs it (F2), nothing a seal holds can change between `BOARD` and `STARTUP` (selection is
  closed), and moving it would re-read the tape-order pins (`tests/arena.rs:6540`,
  `crates/mushd/tests/arena_story.rs:1420-1500`) for nothing.
- **C-3 Profiles M6's (`arena/core.rs:163-164`) or M5's (§22).** Ruled **M5's** (OQ-3); the comment is
  corrected in M5.2; `CoreProfile.profiles` is read at the SEALED mind's class, not the core's (OQ-3:
  Class I → 2, so a line_tank's Class II core does not make it 3).
- **C-4 The gate's "no mind named after the start or in the record" against the `Your mind` block,
  `MIND` and the lead.** Ruled by §12's fuller sentence, "no mind … is named **to anyone else**" (doc
  17:937-938): the pilot's own `SIM MIND` read, seal line, `MIND` and `Your mind` block name their own
  mind (OQ-8, OQ-9); the account that set a side's fill reads it with `SIM AI MIND` after the start too
  (it is that account's own choice, and §14 makes the side lead the fill's trace egress, doc
  17:1048-1052); "the record" is the record other players read — the board from `Boarding` on,
  `SIM HISTORY`, `SIM REPLAY`, another pilot's `SIM REPORT`; staff are not "anyone else" (D10: the staff
  fold's `mind="…"`, `match_/report.rs:1285`, `@mind/set`'s read-back, §14's trace egress to staff), and
  `MatchRecord.minds` and the tape's mind lines keep naming the mind by design (`match_/record.rs:434-448`).
- **C-5 "`a mind` or nothing" against "no mind on a new slot" (§15 :1129, §23 :1741) and the built
  default seal.** Ruled by OQ-9 (i): an unchosen slot stays as built (sealed Standard Doctrine on Line,
  which flies a released hull); the board prints nothing for it and `a mind` for a chosen one; §15's
  and §23's "no mind" read as "no mind of the pilot's own", corrected in the record.
- **C-6 "clicking a rule on the web prefills `MIND PROFILE …`" (doc 17:985-986).** Ruled: a **profile**
  row's click prefills `MIND PROFILE <name>` and sends nothing until Enter; M5 seals no rule, and what a
  rule row's click prefills is the cut that seals rules (M6, C-22).
- **C-7 Q-UM-15's two sentences when the sealed mind IS Doctrine; the fill's preset.** Ruled by OQ-5:
  Doctrine is Standard Doctrine at any preset; a pilot's sealed Doctrine profile flies on release in
  every match. Under the default, a lead's NON-Doctrine fill choice flies "Doctrine at the lead's chosen
  preset" (doc 17:1488): Standard Doctrine at the lead's chosen profile when it is one of Doctrine's six
  presets, else Line; a pilot's non-Doctrine choice goes to "Doctrine on its fallback preset" (:1489-1490),
  Line (no fallback-preset configuration exists, critic G10). Both are taken AT THE SEAL (M5.2 item 11;
  a declared departure, §7 4).
- **C-8 "`SIM MIND` prints the arithmetic" (Q-UM-2 A, doc 17:1439) against the bare sample (:926-927).**
  Ruled: both — a refusal prints doc 17 §9's sentence in OQ-11's wording, and the bare read adds one
  line of the side's half and what is spoken for, the pool "visible in the fiction".
- **C-9 17b:60 against doc 17 §22** ("the hints", "all three clients", "bot reader", no "local rig").
  Ruled: doc 17 is the authority (17b:5-6); 17b's M5 cut-map row is revised by this note's commit to
  §8.2's text.
- **C-10 The index calls itself "the one list" but omits the earlier cuts' deferrals.** Ruled: §1.3
  is the complete list; the M5 record says the index was not.
- **C-11 §12's board reads `flown by its mind` "or on `EXIT`" (doc 17:963-966); built `, left`.**
  Ruled: a drop reads `, flown by its mind`; an `EXIT` keeps `, left` — the reader's and the bot's
  "cannot take it back" (`Standing::Left`), the hand-over said once in `EXIT`'s own sentence ("… is on
  its own for the rest of the match; you cannot take it back.", `match_/cockpit.rs:2316-2320`). A
  declared departure (§7 2). **C-11a** (§22's italic "the cockpit's sentences speak of the sortie where
  a match is not meant"): in the arena the sortie and the match end together (§12 :917-919), so a
  sentence about the hull's mind or its seal says *sortie* (the seal line) and one about the seat or the
  result says *match* (`EXIT`'s "for the rest of the match", `ROM_TAIL`'s "MIND is off for the match",
  `arena_bridge.rs:4854`, §8's arena form); both stand.
- **C-12 §15's first hint "by your rules"** (doc 17:1124-1126) is false in M5 (no rule is sealed: OQ-4).
  Ruled: the hint is reworded to what M5 makes true (M5.6), §15 corrected in the record.
- **C-13 "with no pin the latest saved revision is resolved at each `SIM START` and printed then"
  (doc 17:928-929).** Vacuous (no revisions before M6, OQ-2); the seal line before `STARTUP` is the one
  line "printed then", and `SIM START` says only Q-UM-15's sentence, only when a substitution happens.
- **C-14 "its first six hex digits are the seal id players see" (doc 17:1102-1104).** No seal id is
  printed in M5: the id is a revision's `DefinitionDigest`, M6's; six hex of `MindSeal::digest` would be
  a new definition, and every pilot-facing surface is swept for sixteen hex (F12) (critic G12).
- **C-15 §9's example (96,000 / 100,000 / 92,000)** adds up under neither reading. Ruled by OQ-11; the
  example is corrected in the record.
- **C-16 §9 :755 "`SIM UNIT`'s check counts no fill and cannot refuse before M5"** is stale since M4.3
  (it counts the fill, `match_/pool.rs:38-41`); after M5.1 it counts seated hulls too. Corrected in the
  record.
- **C-17 `SealRef::doctrine`'s doc and the design map's §2.16 ("a `MindSeal` shape change moves every
  seal digest").** True of an always-written field only (F4); M5.2 corrects the comment.
- **C-18 §12's "The station view says so in one line before `STARTUP`" for every seal.** Ruled: said to
  a seated pilot whose slot chose a mind (or whose choice was substituted); an unchosen slot's seal is
  the default the board, the help and `MIND` already describe (OQ-9 (i): "stays as built"), so every
  existing wire transcript and the bot's e2e lane are unchanged. A declared departure (§7 1).
- **C-19 §11's "Picking a preset copies it into the profile; the card then says `Line, edited
  (min_odds 60)`" (doc 17:899-900)** — editing is M6's (OQ-3: no slider editing); vacuous in M5.
- **C-20 D-UM-44's "a recorded intent at most once per 10 s"** — no intent exists (F6). Ruled:
  `IntentKind::Profile`, seat-only, on helm, continuous under a key of its own, journalled as
  `mind_card` (§3, M5.2).
- **C-21 "M5's authored minds"** (doc 16:1139 "`fill_class` … caps nothing until M5 rates minds", "an
  authored mind's draws (M5)"; doc 17:1800 "declared for M5's authored minds"; :1805 "M5, which fields
  other minds"; 17b:115's deviations) — nobody authors before M6 (doc 17:1603-1611). Corrected in the
  record: M5 fields other minds only through the test seam; rating above Class I is the cut that rates a
  mind above it.
- **C-22 §12's card drawing and rule glyphs (R-24, R-25; doc 17:983-988).** "The card draws the same
  way everywhere, one slider a line with a 10-cell bar and the unit-aware value" is the schema-rendered
  card of M6 (17b:61: "schema-rendered cards, the web workshop, the TUI card screen"), not the six-row
  block, which cannot hold a card's twenty-odd sliders; **ruled: M5 draws no card on any surface**, so
  "the same way everywhere" holds vacuously and lands with M6. "The first three rules carry a state
  glyph": no rule is sealed on a profile before M6's authoring (OQ-4; §6.2), so the glyphs have nothing
  to mark and land with the first sealed rule (M6). **The ruling that keeps the card drawing out of
  the block is OQ-8** ("the block shows the seal facts, profile, channels and rules"), not OQ-3's "no
  slider editing", which is about editing, not drawing (the draft cited OQ-3). A reading, so on §7's
  veto list (5), and carried in 17b's Deviations as a residual still open.
- **C-23 R-60 against the seal's floor probe.** `seal_for` runs the Compact-floor probe on every hull
  (`match_/seal.rs:255-272`), so an operator floor above 100 refuses an unarmed drone's Line card and
  calls the match off — against §11 :864-866 ("shown inert with the reason … and never refuses the
  seal"). **Ruled:** the floor probe (the fire section's `min_odds`) runs only on a hull with a weapons
  station (`channels` holds `Fire`); the card's `pair` (helm: `fight_home_pct` against
  `break_off_pct`, `arena/card.rs:839-848`) still runs on every hull. Applied at the choice (M5.1's fit)
  and at the seal (M5.2). A behaviour change for an unarmed hull under a raised floor (it no longer calls
  a match off), recorded; no existing test pins that refusal on an unarmed hull (the `CardUnlawful` pins
  name red's first pod, a line tank: `tests/match_lifecycle.rs:8889`, `:9866`). The reading M5 relies
  on for what such a hull then flies is **Q-UM-12's default** (D-UM-48, doc 17:1469-1472: Standard
  Doctrine with its fire half absent), which the owner has not ruled otherwise.
- **C-24 The over-class rule, exactly.** Doc 17 §9 refuses "a mind rated above a governed core's
  class". **Ruled:** over-class is `read_core(&unit.unit) == Some(hw) && hw.governed && rated >
  hw.class` — a hull with NO core flies the ROM and is never over-class, whatever `seal_for` records as
  `governed` for it (F15). Read literally on `seal_for`'s `(governed, core_class)` pair, the rule would
  refuse every coreless hull sealed rated I, `SIM RANGE`'s `target_mk1` among them: every range match.

### 2.2 The critic's decisions (§7 D1-D18), each taken

- **D1 the split** — taken, as **seven** sub-cuts: the critic's M5.1 to M5.6 and M5.8, renumbered
  M5.1 to M5.7; its M5.7 (the rule language) is out (OQ-4); the residues are a separate cut (OQ-6).
- **D2 where the profiles live** — taken: the sealed set on `SealRef` (unfolded, per-match constant),
  the active profile on `UnitState.card_profile` (folded only when not the default); one format move in
  M5.2. Both rejected alternatives stay rejected (dormant rows at check-out move a production pin, F5;
  the world handing the arena the card at the release makes the world a source of sealed content
  mid-sortie, against P4 and AF.3's "never re-derived", `match_/record.rs:438-441`).
- **D3 `MindSeal.profiles` skipped when empty** — taken, as names with each profile's card digest
  (`SealedProfile { name, card_digest }`), empty exactly when the sealed set is the Line card alone;
  pinned by a test that a default seal digests to its `5f505c8` bytes.
- **D4 one format move** — taken: M5.2 carries the profile set, the active profile, STOP E's charge and
  the `af1` walk; `FireCondition` does not move in M5 (OQ-4).
- **D5 `MIND PROFILE` as a seat-only intent on helm** — taken, refined: **continuous under a key of
  its own** (`continuous_code` 5; latest wins per `(unit, 5)` in the pre-pulse buffer, so at most one
  lands a pulse and a `HEADING` or `ORDER` of the same pulse is never swallowed), handled in its own arm
  of `apply_intent` BEFORE the continuous gates (a cold or immobile hull switches); the ten seconds
  checked at the arena's landing against a carried, folded stamp; journalled as the existing
  `mind_card` kind; no station function, no `API_VERSION` or `MIND_ABI` move (F6).
- **D6 `FRAME_VERSION` held at 3** — taken (F7), with `Frame::same_as` comparing the new field.
- **D7 the seal at check-out; the line at `run`'s success arm, no seal id** — taken, refined by C-18;
  told to the pilot alone through an `Audience` (F2), never `sys_to_actor`.
- **D8 `SIM MIND`'s grammar** — taken, with `none` to clear, `@<rev>` refused, and the bare reads ruled
  per state (§3 M5.1 item 2).
- **D9 staff set the fill to Doctrine presets only** — taken (staff hold no holdings, D-UM-49).
- **D10 the fog's reach** — taken (C-4), with the fill's setter named.
- **D11 the board and sentence words** — taken (C-11; M5.3).
- **D12 the gate on `sim_solo.rs`, no new ledger kind** — taken; the rig's own 10 s unattended end is
  kept and used (§4).
- **D13 the bot never types `SIM MIND` or `MIND`** — taken (OQ-1).
- **D14 `@mind/set` survives** as the operator's lever (staff only, journalled `mind_card`), recorded as
  not an author's edit, so P4's "edits change nothing aboard" stands for authors; a staff edit on a
  pilot's hull appears in that pilot's `Your mind` block by its card's own name (M5.6).
- **D15 the pool's call-off** — discharged at M4.3 (F13); M5.1 names the refused hull in the long form.
- **D16 `MindSeal.card` stays a JSON integer array** — taken, recorded as still owed.
- **D17 `MindDropped.fault`** — taken: the `Your mind` block prints the cause word only; still open.
- **D18 sequencing** — taken: M5 moves no production pin, so it is independent of T-104; the residues
  cut is not (§6.1).

### 2.3 The design map's open questions, closed

Q-1, Q-2 (the block's data; what it says seated) by OQ-8; Q-3 (holdings) by OQ-2; Q-4 (the unchosen
slot) by OQ-9 (i); Q-5 (where profiles are authored) by OQ-3 (presets; no authoring before M6); Q-6
(profiles by which class) by OQ-3 (the mind's); Q-7 (`MIND PROFILE`'s mechanism) by D5 (a mind does not
switch its own profile: no station function); Q-8 (the fill's class) by OQ-10 (b); Q-9 (`Your mind`'s
content) by OQ-9 (ii); Q-10 (the seal line) by D7 and C-18; Q-11 (bare `SIM MIND` outside `Selection`)
by D8 (a read in any state, to the slot's own character, per state); Q-12 (the board's release word) by
D11; Q-13 (the second hint) by OQ-10 (a); Q-14 (help or behaviour) by OQ-6; Q-15 (staff and the record)
by D10; Q-16 (the engine of a chosen mind; the inert sliders) by OQ-1 and OQ-10 (e) — a Doctrine seal is
`PRODUCTION_FILL_ENGINE`, a fixture's `Scripted`; Q-17, Q-18 (slots across seats; precedence) by OQ-4
(no sealed rules in M5: slots stay cleared on every transition, M1b.1's rule); Q-19 by OQ-11 and F13;
Q-20 by OQ-10 (c); Q-21 by M5.2's gate and M5.3's D14; Q-22 (the lead's choice and the fill's persona
leader) — nothing the persona's deliberated orders assume reads the card (orders steer the objective;
the card is the mind's), recorded as checked by M5.2's fill fixture. Q-UM-15's sub-questions: (a) "the
other side has a human pilot" is decided once at `SIM START` from the claimed slots; (b) Doctrine at
any preset is Doctrine (OQ-5); (c) the sentence is said only when a substitution happens (OQ-5); (d)
`SIM RANGE` has no fill and refuses `SIM AI MIND`.

### 2.4 The fog, ruled per state (C-4, D8, D10; the adversarial review's finding 3)

| read | `Idle` | `Selection` | from `SIM START` on (`Boarding`, `Running`, after) |
|---|---|---|---|
| the board, a slot's row | — | its own character and staff: `, mind Standard Doctrine (Sniper, Line)`; everyone else `, a mind` against a chosen slot, nothing against an unchosen one | nothing, for every reader, staff included |
| the board, a side's fill row | — | the side's lead and staff: `  red fill: Standard Doctrine (Sniper)`; everyone else `  red fill: a mind`; no row for an unchosen fill | nothing |
| bare `SIM MIND` (always the actor's own) | `You hold Standard Doctrine (six presets); claim a slot and SIM MIND lists what fits its core.` | the slot's line, one `Fits` line per held mind that fits, the pool line | `Yours: Standard Doctrine (Sniper, Line); selection is closed.` (or `Yours: none; …`) |
| bare `SIM AI MIND` (one line a side) | as `Selection`, no fill set | the side's lead and staff: the name, or `nothing chosen; it flies Standard Doctrine on Line`; everyone else `Red's fill carries a mind.` or `Red's fill: nothing chosen.` | the account that set the fill and staff: `Red's fill: Standard Doctrine (Hunter).`; everyone else `The fills are not shown once a match starts.` (names nothing, marks nothing) |
| the seal line, `MIND`, `Your mind` | — | — | the pilot whose hull carries the mind, alone |

`SIM AI MIND` answers `SIM AI`'s server facts first, in `SIM AI`'s own order (`match_/lobby.rs:2874-2905`):
with `[tacsim.match].ai_fill` off, every form is refused `SimError::AiOff` (`AI fill is off on this
server.`); on a range (no fill there), the bare read says `AI fill: none on a range.` (`SIM AI`'s own
read) and a set is refused `SimError::AiOnRange` (`The range fields no machines; SIM RANGE sets its
targets.`) — never `AiOff`, which is false where the fill is on.

A side's fill choice is recorded with the account that set it (`FillChoice.by`). **A change of lead
before the check-out clears it**: whenever `resettle_leads` changes a side's lead account (at `SIM
LEAVE`, the boarding deadline or a reset), that side's entry is cleared unless staff set it, so the fill
reverts to Standard Doctrine on Line (the default the board prints nothing for), and no new lead reads
or inherits another account's choice. After the check-out the entry is kept as sealed, read only by its
setter and staff.

---

## 3. The sub-cuts, in order

**The gates under every sub-cut** (17b §0, and the conventions brief `briefs/M5-conventions.md`):
`git fetch` first; `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D
warnings`; strict rustdoc (`RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps
--document-private-items`); the workspace green (`cargo test --workspace --exclude mush-bot
--no-fail-fast`, then `cargo test -p mush-bot` on its own — every sub-cut changes a line-mode sentence
or the keyframe); `cargo test -p mush-contract-tests` (the verb stanzas, the help contract, the arena
library's forbidden list and the panic budget); **the web lane (`npm ci`, `npm test` from `web/`) in
every sub-cut that touches `web/` or a fixture under `web/src/lib/fixtures` — M5.2, M5.3, M5.4 and
M5.5 — and `npm run build` in M5.5**; the five production pins run and read at their `5f505c8` values
BEFORE any re-bless; one commit and one push per sub-cut, by the orchestrator after review; a contract
gate re-blessed in the commit that moves it; a non-baseline `legacy-handlers.toml` stanza for the new
verb; no pin weakened — a pin that moves is re-pinned old → new with its reason in the commit body, and a
production pin that moves is a STOP for the owner. No sub-cut deploys (OQ-1). The production pins named
throughout are the five of 17b's M4 record: `DUEL_HASH_AT_HEAD` (`tests/motor.rs:59`),
`M1B0_DUEL_LINES` (`tests/motor.rs:159`), `M1B0_J_LINES` (`tests/reflex.rs:1378`),
`m1a5_the_production_hash_does_not_move` (`tests/minds.rs:3158`) and `M2_CROSS_PLATFORM_PINS`
(`tests/minds_rhai.rs:3255`). M5.2 and M5.4, which add per-unit and per-frame work, also run the release
harness lanes once and the delta WITHOUT bless, which must read every block equal (17b §0.5).

**Why this order, and each gate in exactly one sub-cut** (the M4 row's (vi) form): the lobby first,
because every later sub-cut reads the choice and M5.1 touches no arena state; the seal second, because
it holds the milestone's ONE format move, `af1`'s one re-bless and every carried field — the sealed set,
the active profile and its intent, STOP E's charge — so no later sub-cut moves the format; the release's
words third, the bot's resume reader moving once; the verb fourth, with the one keyframe key and the
canonical fixtures' one re-bless; the renderers fifth, with the goldens' one re-bless each; the report,
the hint and the help sixth, with the hint counts and the pack's version; the gate and the record last.
The bot's reader moves in M5.1 (`, a mind`), M5.2 (the `profile` word, two neutral lines), M5.3 (the
release words), M5.4 (the `MIND` lines) and M5.6 (the reworded acknowledgements), each a declared
reader change with `cargo test -p mush-bot` in that sub-cut's gate.

### M5.1 `tacsim:` the choice in the lobby — `SIM MIND`, `SIM AI MIND`, the holdings seam, the fit, the pool, the selection fog

**Contents.**

1. **The holdings seam** — `match_/holdings.rs` (new):
   - `SealIdentity { mind, charter, line: SmolStr, revision: u32, source_sha256: [u8; 32] }`
     (`match_/seal.rs`, beside the `SEALED_*` re-exports): exactly the five identity fields `seal_for`
     writes (`match_/seal.rs:286-293`); `SealIdentity::doctrine()` = `SEALED_MIND`/`SEALED_CHARTER`/
     `SEALED_LINE` (`arena/doctrine.rs:351-363`), `crate::minds::STOCK_VERSION`, `standard_digest()`.
     No behaviour change in M5.1; M5.2 puts it on `SealTerms`.
   - `HeldMind { identity: SealIdentity, rated: CoreClass, engine: SealEngine, presets: Vec<SmolStr>,
     source: Option<Arc<str>> }` — `source` `None` for Doctrine (its text is compiled in and stored at
     boot), `Some` for a scripted fixture.
   - `pub trait Holdings: Send + Sync { fn held(&self, w: &World, character: ObjId) -> Vec<HeldMind>; }`.
   - `DoctrineOnly`, the production provider: every character holds exactly `HeldMind::doctrine()` —
     `SealIdentity::doctrine()`, `STOCK_RATED_CLASS` (I), `PRODUCTION_FILL_ENGINE`, the six
     `card::PRESETS` (`arena/card.rs:409`), no source. Keyed by the character who boards (`Slot.pilot`;
     Q-UM-13's default, confirmed by OQ-2); the lead's own slot's `pilot` for the fill (F11).
   - `HoldingsSeam(Arc<dyn Holdings>)`, tacsim module state installed with `DoctrineOnly` by
     `tacsim::register`; `pub fn install_holdings(w: &mut World, holdings: Arc<dyn Holdings>)` for test
     rigs — a production seam with a test provider, `install_sealed_runners`' precedent, not a fixture
     impersonating a production module (17b §0). **It installs each held `Some` source into the
     store's RAM map** through a `pub(crate) fn install_mind_source(store: &mut TacsimStore, text)`
     split out of `store::put_mind_source` (which keeps calling it and then queues the document), so
     `minds::source_lookup` (`arena_bridge.rs:1540-1559`) finds a fixture's source exactly as it finds a
     stored one, and `open_match` never answers `InstallError::NoSource` for it (F1). Integration rigs
     cannot call `put_mind_source` (`pub(crate)`, needs a `Ctx`); this is their one path.
   - **The test provider** (`tests/support/holdings.rs`, tacsim's integration tests only; the mushd fog
     battery needs none, since `DoctrineOnly` already refuses every other name): Doctrine plus two
     scripted fixture minds —
     - **`Fixture I`** — `tests/support/fixture_i.rhai` (NEW, a few lines: `startup()` until online,
       then `hold()`; well under Class I's 1,500 ops), rated I, `SealEngine::Scripted`, a non-Doctrine
       identity (`charter` and `line` not the Registry's, so `SealRef.doctrine` reads false), presets
       `Line` and `Sniper`, its digest pinned in the provider. **It is the fixture of every non-Doctrine
       path**: the seed gate through the lobby, Q-UM-15's substitution, D14's catch-up, the keyframe's
       `"name": null`, and the not-held/fit/pool cases. Every test fielding it fields a `line_tank`
       (Class II line core) at the default `fill_class` I, so it seals at Class I and thinks inside the
       budget.
     - **`Load III`** — the existing `tests/support/load_iii.rhai` (digest pinned at
       `tests/harness.rs:9473-9490`; not copied), rated III — used ONLY by
       `m5_an_over_class_mind_is_refused_at_the_choice` (on a `line_tank`). On the heavy it would seal at
       Class I and fault `ops` every think (F15), so no test flies it through the lobby.
     (Answer to the feasibility review's "don't create a new file": that was about not copying Load III;
     a rated-I mind is a different fixture the adversarial review found missing.)
   - Resolution: case-folded; `standard`, `standard doctrine` and `doctrine` name Standard Doctrine
     (the first hint types `SIM MIND Standard`, critic G7); any other held mind by its own name.
2. **`Selector::Mind`**, the fourteenth (`match_/sim.rs:127-141`: `ALL: [(Selector, &str); 14]`,
   appended as `range` was; `register_switches` gives `SIM/MIND`). Grammar (D8):
   - `SIM MIND` — a read, in any state, to the actor alone; its lines per state are §2.4's.
   - `SIM MIND <mind> [PROFILE <p>[, <p>]…]` — `Selection` only, the actor's own slot; `PROFILE`
     case-folded; profiles comma-separated, the first the one that flies at the release, the rest
     switchable with `MIND PROFILE`; no `PROFILE` means Line alone. A set (or `none`) reuses `SIM
     UNIT`'s refusals in `SIM UNIT`'s order (`match_/lobby.rs:1422-1480`): outside `Selection`,
     `SimError::WrongState` (`The simulator centre is {state}; …`); with no slot claimed,
     `SimError::NoSlotToField` (`Claim a slot first: SIM TEAM red|blue.`, `SIM UNIT`'s precedent: the
     list is the reason to want a slot) — no new variant for either case; item 3's refusals are the
     new ones.
   - `SIM MIND none` — clears the choice (`Selection` only).
   - `SIM MIND <mind> @<anything>` — refused (`@rev` pins wait for M6, OQ-2).
   - `SIM AI MIND …` is parsed in the `Ai` arm when the rest's first word is `mind`, case-folded,
     BEFORE the `u8` parse (`match_/sim.rs:234-248`): bare `SIM AI MIND` reads (§2.4, per state);
     `SIM AI MIND <mind> [PROFILE <p>]` and `SIM AI MIND none` set — the side's lead for their own side,
     or staff for any side, Doctrine presets only for staff (D9); `Selection` only; exactly ONE profile
     (nobody aboard a fill hull switches it); in `SIM AI`'s order (§2.4): `SimError::AiOff` when
     `[tacsim.match].ai_fill` is off, `SimError::AiOnRange` for a set on a range (the bare read there
     says `AI fill: none on a range.`), `SimError::WrongState` outside `Selection`,
     `SimError::NotLeadForAi` for an account that leads no side and is not staff.
   - `USAGE` (`match_/sim.rs:195-198`) and the `SIM` help string (`:513-574`) gain both rows.
3. **The sentences** (each the reply to the actor's own command, so `sys_to_actor(…, NOTICE_CATEGORY,
   …)`; never `announce`, `match_/lobby.rs:4098-4115`, which tells every centre room and pod — `SIM
   UNIT`'s announce is what `SIM MIND` must not copy; each line under 120 columns, measured at the
   widest preset pair and the longest chassis name, `line_tank_thermal`):
   - bare, unchosen: `Yours: none; a tank you leave flies Standard Doctrine on Line.` (62);
   - bare, chosen: `Yours: Standard Doctrine (Sniper, Line).` (40; at the widest pair 63);
   - bare, one line per held mind that fits the slot's chassis, the count read at the sealed class:
     `Fits line_tank_thermal (II): Standard Doctrine, up to two of Line, Sentry, Sniper, Skirmisher,
     Hunter or Rearguard.` (115 at the longest stock type word; measured 2026-10-11 over every unit
     the stock content fields: `line_tank_laser` 113, `line_tank_eccm` 112, `heavy_mk1 (III)` 108, the
     rest 106-111; the draft's `(…), two profiles at most` form ran 123 on `line_tank_thermal` and 121
     on `line_tank_laser`); on an unarmed chassis the same line (every preset is lawful there, C-23).
     The width is pinned by a test over every `fieldable_ids` type word as `type_word` prints it
     (`match_/lobby.rs:1206-1243`; the full canonical id where a short name is not unique, which no
     stock pack makes);
   - bare, the pool: `Red's half of the centre's pool is 238,000 ops a second and 3,000 are spoken for.`
     (81; C-8);
   - bare, outside `Selection`: §2.4's lines (`Yours: …; selection is closed.`, ≤ 70);
   - bare, no slot: `You hold Standard Doctrine (six presets); claim a slot and SIM MIND lists what fits
     its core.` (93);
   - set: `Your slot will carry Standard Doctrine (Sniper, Line).` (≤ 63); cleared: `Your slot carries no
     mind of your own; a tank you leave flies Standard Doctrine on Line.` (88);
   - refusals: `You hold no mind called Sentinel; SIM MIND lists the minds you hold.` (68, posts
     nothing); `No revisions are kept before the workshop; SIM MIND Standard seals the Registry's own.`;
     `Standard Doctrine has no profile called Brawler: Line, Sentry, Sniper, Skirmisher, Hunter or
     Rearguard.` (103); `Sniper is named twice; a profile is sealed once.`; `Standard Doctrine thinks at
     Class I on this core, which seals two profiles at most.` (83); `Skirmisher fires at 30% and better;
     this server's Compact floor is 40%, so it cannot be sealed.` (95, an armed chassis only); `Load III
     thinks at Class III; this line_tank_thermal's core is Class II and is governed.` (88, fixture only);
     the pool's (item 8);
   - `SIM AI MIND`: set `Red's fill will carry Standard Doctrine (Sniper).`; the bare reads §2.4's;
     refusals `The fill flies one profile; nobody aboard switches it.` and `Staff hold no minds; SIM AI
     MIND Standard sets the fill's doctrine and preset.` (78).
4. **The fit at the choice** — one `fit_mind(w, team, slot_type, held, profiles) -> Result<CoreClass,
   SimError>`, so `SIM MIND` and `SIM UNIT` ask the same questions and "the later … is the one
   refused" (doc 17:933-934):
   - the chassis: `slot.unit_type`, else `authored_unit_type` (`match_/lobby.rs:1279`); its core read
     WITHOUT an instance by **`core_of_type(w, id) -> Option<(CoreClass, bool /* governed */)>`** beside
     `class_of_type` (`match_/lobby.rs:1293-1301`): the unit spec names its core through
     `ComponentRole::Core { core }` (`definitions/unit.rs:871`) and the store holds each `CoreDef` with
     its class and governed bit in `definitions().cores` (`definitions/mod.rs:238`); and
     **`armed_of_type(w, id) -> bool`** beside it (the spec has a weapon component), for C-23;
   - the class: `min(held.rated, core, [tacsim.minds].fill_class)` (the seal's rule, `seal.rs:280-283`);
     a chassis with no core: `Zero`, never over-class (C-24);
   - **over-class** (D-UM-41, OQ-10 (d), C-24): `core == Some((class, true)) && held.rated > class`
     refused (reachable with `Load III` only);
   - **the profile count**: at most the class row's `profiles` (`arena/core.rs:163-164`; Class I → 2,
     OQ-3); each a preset of `held.presets`, no duplicate;
   - **the floor**, on an armed chassis only (C-23): each preset's card (`card::preset(name,
     &arena_cfg.reflex)`) lawful against the operator floor by the probe `seal_for` runs
     (`match_/seal.rs:255-272`), refuse-not-clamp (D-UM-17); the card's `pair` on every chassis;
   - **the pool**: `pool_at_the_choice(cfg, per_side, others, Some(class))` (`match_/lobby.rs:1606`)
     with `others` = the side's other claimed slots at THEIR class (chosen, or `fill_class` unchosen)
     and its unclaimed pods under the fill at the fill's chosen class or `fill_class`;
   - `SIM UNIT` (`match_/lobby.rs:1422-1604`) runs `fit_mind` for the slot's standing choice after
     resolving the new type, and passes its own class (not `None`) to the pool.
   - **Who re-runs the pool at the choice**: `SIM UNIT` and `SIM MIND` only. `SIM AI <n>`, `SIM AI
     MIND` and a claim (`SIM TEAM`) do not: the check-out's gate (`StartError::Pool`) is their
     backstop, as at `5f505c8`. Neither a claim nor a fill choice can raise a side's load: a claim
     turns a fill pod admitted at `fill_class` into a seated hull admitted at `fill_class` (item 7),
     and a chosen class is `min(rated, core, fill_class)`, never above the `fill_class` an unchosen
     hull is admitted at; `SIM AI <n>` changes the count, bounded by `max_slots_per_side`. None can
     refuse at the stock constants (twelve Class III a side fit, `match_/pool.rs:31-41`).
5. **The state** (each `#[serde(default)]` under `LIVE_FORMAT`'s policy, `match_/record.rs:45-52`, the
   `released_by_resume` precedent `match_/lobby.rs:126-137`; cleared with the board at every return to
   `Idle`, `release_rows` `:3732`, `to_idle` `:3634`):
   - `Slot.mind: Option<MindChoice>`; `Centre.fill_minds: Vec<FillChoice>`;
   - `MindChoice { mind: SmolStr, profiles: Vec<SmolStr> }` — canonical spellings; empty `profiles` is
     Line alone; `FillChoice { team: Team, by: FillSetter /* Lead(AccountId) or Staff */, choice:
     MindChoice }`;
   - `Centre::resettle_leads` clears a side's `FillChoice` set by `Lead(a)` when that side's lead
     account is no longer `a`, before the check-out (§2.4).
6. **To the check-out**: `ready()` (`match_/lobby.rs:2571`) resolves every fielded hull's choice
   against the seam — a holding the character no longer holds refuses the check-out with the new
   **`SimError::MindNotHeld { team: &'static str, index: Option<u8> }`** (`ready()` returns
   `SimError`, not `StartError`), `Slot red 1's chosen mind is no longer held by its pilot.` or `The
   red fill's chosen mind is no longer held by its lead.`, which `run`'s `ready` arm calls off as
   every `ready` refusal is (`Match 1 could not start: Slot red 1's chosen mind is no longer held by
   its pilot. The pods are free again.`, 16b cut AE.3, `match_/lobby.rs:2439-2455`); the sentence names
   the slot, never the mind, because the call-off is said to the centre (C-4); unreachable in
   production (`DoctrineOnly` holds Doctrine for every character), reached with the test provider —
   and writes `Fielded.mind_class = Some(class)` for a hull whose slot, or
   whose side's fill, chose (the three literals `:2633`, `:2657`, `:2704`; an unchosen hull keeps `None`
   and is admitted by the fill rule, item 7); `Side` (`match_/mod.rs:774-780`) gains `minds:
   BTreeMap<ObjId, SealChoice>` (`SealChoice { held: HeldMind, profiles: Vec<SmolStr>, chose: bool,
   substituted: bool }`, `substituted` false until M5.2). **Declared: the choice is recorded and read
   back; the check-out still seals Line until M5.2.**
7. **Seated hulls at the pool**: `pool::admitted_class(chosen, fill_class)` — a hull with no chosen
   class is admitted at `fill_class`, seated or not (a release hands it to a mind of that class); the
   `seated` parameter goes; the module head (`match_/pool.rs:23-29`) and `Fielded.mind_class`'s doc
   (`match_/mod.rs:749-756`) updated. At the stock constants neither gate can refuse (twelve Class III a
   side fit, `match_/pool.rs:31-41`).
8. **OQ-11's wording**: `PoolRefusal`'s `Display` (`match_/pool.rs:89-100`) becomes `This tank would
   think {would} ops a second; its side's half of the centre's pool is {half} and {spoken} are spoken
   for.` (117 columns at six-figure terms); `StartError::Pool` (`match_/mod.rs:882-888`) gains the
   refused hull (`unit: ObjId`, `callsign: String`, `CardUnlawful`'s shape; `admit_side` returns its
   index) and renders `{callsign}: This tank would think …` — the long form, the log's and the
   fixtures', which no pilot reads; the call-off short form (`:913-917`) stands.
9. **The selection fog on the board** (`status_lines`, `match_/lobby.rs:3776-4045`), §2.4's two board
   rows, in `Selection` only, after the `[type]` bracket and on the bracket's model (`:3928-3946`); the
   `AI fill: each side fields …` row (bot-parsed at `lines.rs:995-997`) is untouched; from `Boarding` on
   no row names or marks a mind for any reader (doc 17:937).
10. **Rider: the one-row class.** `validate_classes`' `rows` floor 1 → 2 (`config.rs:923`), with a boot
    refusal test (the `between` sentence, `must be 2 to 32 (it is 1)`).
11. **The bot** (declared reader change): `board_pilot` (`crates/mush-bot/src/arena/lines.rs:1031-1072`)
    strips `, a mind` before the `[type]` bracket into `BoardPilot.mind: bool`; the fill row reads
    `Line::Other`; `SIM MIND` joins `arena_e2e.rs`' `forbidden` (`crates/mush-bot/tests/arena_e2e.rs:125-132`;
    `MIND` joins it in M5.4).
12. **Comments made true in files this sub-cut edits** (F16): `crates/mush-config/src/config.rs:103`
    and `:1409`; `config.rs:709`, `:825`; `match_/lobby.rs:582`, `:1420`, `:1534`, `:1538`, `:1594`,
    `:1599`, `:2632`, `:2656`, `:2703`, `:4133`; `match_/pool.rs:24-39`, `:65`, `:73`;
    `match_/mod.rs:753`, `:858`, `:1179` (check-out's "a seated hull nothing, M5's gap", false once item
    7 lands) — each now says what M5 built, or names "the cut that rates a mind above Class I" (C-21).

**What moves, and why the note allows it.** `match_/sim.rs:588` (13 → 14) and its comment; the `sim`
stanza's "THIRTEEN selectors" (`crates/mush-contract-tests/legacy-handlers.toml:886-889`);
`docs/help/simulator.md`'s selector rows (naming no cockpit `MIND` until M5.4 registers it,
`help_commands.rs:184`); the pool sentence wherever printed — `match_/pool.rs`' tests,
`match_/lobby.rs:4145` (`m3_the_pool_at_the_choice_refuses_with_the_arithmetic`),
`tests/match_lifecycle.rs:9678` (`m3_an_over_subscribed_roster_is_refused_by_name_at_check_out`,
`:9661-9700`: `Err(StartError::Pool { team: "red", refusal: expected })` gains the hull's two fields
and the reworded sentence), `tests/match_lifecycle.rs:9800` (`m3_the_fill_rule_decides_the_pool_…`,
whose check-out arithmetic now counts seated hulls at `fill_class`), and
`crates/mushd/tests/sim_units.rs:1498-1513` (the help sentence check); `sim_units.rs:1466`
`m3_sim_unit_is_never_refused_for_the_pool_before_m5` renamed
`m5_sim_unit_is_never_refused_for_the_pool_at_the_stock_constants`, its doc (`:1455-1464`) and its
assertion (`:1495`) read at the stock pool; the bot's board reader. All are lobby sentences, counts and
the pool's arithmetic, which the scope (deliverables 1, 2; the pool carry; OQ-11) names. **No hash, no
format, no frame, no seal digest, no tape line moves**: nothing here reaches the arena or the seal.

**Gate evidence.** In `tests/match_lifecycle.rs` (the lobby rig; each named with its chassis):
`m5_sim_mind_lists_only_what_the_character_holds_and_what_fits` (`DoctrineOnly`; a `line_tank` and a
`drone_mk1` slot); `m5_a_mind_the_character_does_not_hold_is_refused_and_posts_nothing` (against
`DoctrineOnly` and the test provider; the write list from `board_writes`/`pump_writes` holds nothing for
`SIM MIND`); `m5_the_later_of_sim_unit_and_sim_mind_is_refused_with_the_arithmetic` (under a lowered
pool, both orders, `Fixture I` on a `line_tank`); `m5_an_over_class_mind_is_refused_at_the_choice`
(`Load III` on a `line_tank`; and a coreless chassis never over-class); `m5_a_profile_under_the_floor_is_refused_as_typed`
(a `line_tank`) and `m5_an_unarmed_hull_takes_any_preset_under_any_floor` (a `drone_mk1`, C-23);
`m5_the_sealed_class_caps_the_profile_count`; `m5_sim_ai_mind_is_the_leads_and_reads_to_the_lead_alone`
(and staff's Doctrine-only refusal); `m5_the_bare_reads_follow_the_state` (§2.4's table row by row:
`Idle`, `Selection`, `Boarding`, `Running`, for the owner, the setter, another claimant, a bystander and
staff); `m5_a_change_of_lead_clears_the_sides_fill_choice` (the lead sets, leaves with `SIM LEAVE`; the
new lead's bare read is `Red's fill: nothing chosen; it flies Standard Doctrine on Line.`; the
check-out's `Side.minds` holds no fill choice); `m5_the_board_names_a_slots_mind_to_its_own_character_alone_in_selection`
(the `q8_the_board_prints_a_chosen_type_only_during_selection` analogue, `:1790-1823`: Ada, Bo, a
bystander, staff; then `Boarding`, where nobody reads a mind); `m5_a_seated_hull_is_admitted_at_fill_class`
(`pool.rs` in-source); `m5_every_sim_mind_line_is_under_120_columns_on_every_fieldable_type` (each of
item 3's lines over every `fieldable_ids` type word, the widest preset pair, six-figure pool terms);
`m5_sim_mind_and_sim_ai_mind_reuse_the_lobby_refusals` (`WrongState` outside `Selection`,
`NoSlotToField` with no slot, `AiOff` with the fill off, `AiOnRange` and the range's bare read,
`NotLeadForAi`); `m5_a_holding_lost_before_check_out_calls_the_match_off_by_slot` (the test provider
drops `Fixture I` between the choice and the check-out: `MindNotHeld`, the call-off naming the slot
and no mind). `config.rs`' refusal test. `help_commands` (every `SIM MIND` word resolves, the
`mind` switch registered). `cargo test -p mush-bot` with a reader case for `, a mind` and the fill row.

**Risks.** The `SIM UNIT` → `SIM MIND` re-check order: the type change resolves first, then the
standing choice is fitted, then the pool, so the arithmetic printed is the new chassis's. The pool
sentence becomes player-reachable under a lowered pool only.

### M5.2 `tacsim:` the seal carries the choice — the one `SNAPSHOT_FORMAT` move (9 → 10)

**Contents.**

1. **The seal** (`match_/seal.rs`):
   - `SealTerms` (`:66-102`) gains `identity: SealIdentity` (M5.1) and `profiles: Vec<(SmolStr,
     StockCard)>` (empty = Line alone; `card` stays the first profile's card, the one that flies at the
     release; the fixture terms `:167`, `:202` take `SealIdentity::doctrine()` and an empty list).
     **`terms.rated` and `terms.engine_for_minds` stay the terms' own and authoritative**: `seal_for`
     takes only the five identity fields from `terms.identity`, and `match_::start` copies
     `held.rated`/`held.engine` into `rated`/`engine_for_minds` (item 10). So every fixture lever stays
     whole — `.rated(CoreClass::III)`, the harness's Load III heavies at `rated: III`/`Scripted`
     (`tests/harness.rs:12370-12386`, whose post-seal identity writes `:12391-12394` stand),
     `Muster::scripted`'s Standard Doctrine on `Scripted` (`tests/harness.rs:9854-9858`),
     `SealTerms::fixture`'s `Native` — and no seal digest or mind-fixture hash pin moves by a second
     source of rating or engine.
   - `seal_for` (`:233-310`) writes the identity from `terms.identity` (production: the same constants,
     so the same bytes); validates EVERY profile's card — its `pair` on every hull, and the floor probe
     (`:255-272`) only on a hull with a weapons station (C-23); refuses **`SealError::OverClass { rated,
     core }`** when `read_core(&unit.unit) == Some(hw) && hw.governed && terms.rated > hw.class` (C-24;
     a hull with no core is never over-class) — the backstop behind M5.1's choice; at check-out
     `StartError::MindOverClass`, short form `{callsign}'s mind is over its core's class`, unreachable
     through the lobby; and writes, LAST in the record, `MindSeal.profiles: Vec<SealedProfile { name:
     SmolStr, card_digest: [u8; 32] }>` under `#[serde(default, skip_serializing_if =
     "Vec::is_empty")]` — **empty exactly when the sealed set is the Line card alone**, so every
     default seal (every production seal, every fixture seal) is the `5f505c8` record byte for byte
     (F4); `card`/`card_digest` stay the first profile's;
   - doc comments made true: `SealTerms.card` (`:72-74`), `MindSeal.armed` (`arena/mind.rs:491-492`:
     "none before 17b cut M6, whose authoring seals a profile's rules; the rule-language cut after M5
     arms a pilot's slots, never a seal's"), the record's "before 17b cut M5" field list
     (`arena/mind.rs:441-452`), and `SealRef::doctrine`'s "would move every seal's digest" (`:384-386`,
     C-17).
2. **The sealed set on `SealRef`** (`arena/mind.rs:355-397`): `profiles: ProfileSet` — `ProfileSet {
   len: u8, codes: [u8; 4] }` in `arena/card.rs` beside `PRESETS`, each code an index into `PRESETS`,
   `Copy` (so `SealRef` stays `Copy`), `PartialEq`, serialised as the array of its codes;
   `ProfileSet::LINE` for an empty `MindSeal.profiles`; derived in `MindSeal::seal_ref`
   (`:531-552`) beside `doctrine`; `SealRef::none()` reads `LINE`. Carried by the unit's derive;
   **never folded** — `hash.rs`' "Never folded" list (`:218-223`) names it, a per-match constant as the
   rest of `SealRef` is, and the never-folded probe (`arena/hash.rs:2300-2323`) gains a
   `("seal.profiles", …)` row.
3. **The active profile on `UnitState.card_profile`** (`arena/state.rs:1861-2223`; F5: `profile` is the
   hull's `UnitProfile`): `card_profile: ActiveProfile { index: u8, switched_ms: Option<i64> }`, default
   `{ 0, None }`, in `arena/card.rs`; laid at the default in `Arena::new` (`arena/state.rs:4550-4570`);
   carried — `snapshot.rs`' `every_unit_field` (`:1245-1300`) names it, 43 → 44, decided carried on
   purpose. **Folded as item 1's sub-item 20, only when not the default**: after the pod's switch
   (sub-item 19, `arena/hash.rs:106-112`, the fold `:699`), `[1]`, `index` as `u8` and `switched_ms` as
   `i64` — nothing at all for a unit at its first profile and never switched, so every match that
   switches nothing (every production match, every existing fixture) folds exactly the bytes it folds at
   `5f505c8`. Its two fields decide a later card and a refusal, the reason it is folded (16b cut M.1's
   rule for the seat window, `:98-101`).
4. **`IntentKind::Profile { index: u8 }`** (`arena/intents.rs:121-260`), the intent D-UM-44 names:
   - `channel_required` Helm (`:342-357`, the `Order` precedent); `is_continuous` (`:298-306`);
     **`continuous_code` 5** (`PROFILE_CODE`, `arena_task.rs:980-1032`; the discrete kinds stay 4,
     `GOAL_CODE` 3), so the buffer coalesces per `(unit, 5)` — the desired profile, latest wins, at most
     one landing a pulse — and a `HEADING`, `SPEED`, `ORDER` or `DRIVE` typed in the same pulse keeps its
     own key; `buffer_continuous`'s goal-settling pairs do not involve it; the function's doc gains the
     code's reason; `as_str` `mind` (the verb the pilot types, as `Goal` reads `drive`, `:359-375`; one
     word, as the wire and the ledger expect);
   - **its own arm in `apply_intent`, BEFORE the continuous gates** (`arena/intents.rs:792-990`; the
     `PowerNotOnline` gate `:912` and the `MotionDisabled` gate `:915` run for every continuous order
     but `Order`/`ReleaseOrder`, whose arm returns early at ~`:897`), on `IntentKind::Order`'s
     precedent: the wreck, the seat's epoch, the channel (Helm) and the source checks — accepted from
     `IntentSource::Seat` only; `Mind`, `Machine` and `Cond` refused `Refusal::Channel` (no station
     function builds one, F6) — then the `Refusal::Profile` cases. **A cold or immobile hull
     switches**: the profile is what the mind will fly, not motion, so `power_not_online` and
     `motion_disabled` never answer a switch, and the cockpit's pre-check (M5.4) tests no power;
   - at its landing: refused **`Refusal::Profile`** (new; word `profile`, "that profile cannot be
     switched to now") on a hull with no seal (`SealRef::none()`), an `index` not under
     `seal.profiles.len`, the profile already active, or a landing inside 10,000 ms of `switched_ms`
     (D-UM-44's "at most once per 10 s") — the `NoJammer` precedent (`:526-531`): the cockpit refuses
     each case locally (M5.4), and this is the arena's word when an order gets that far. The word is
     outside the mind's API (F6), so `API_VERSION` stands;
   - on success: writes `UnitState.card_profile`, and over a STANDING (dormant) mind row
     `row.card = card::preset(PRESETS[code], &arena_cfg.reflex)`; journals the existing
     `EventKind::MindCard { unit, name, changed, card }` (`arena/ledger.rs:539-550`; `changed` the
     sliders the switch moved, `@mind/set`'s computation) — a recorded intent (§11), on the staff fold
     and, from M5.6, the pilot's own. **`MindCard`'s doc is amended** (`:527-538`): "a staff
     `@mind/set` on the arena's own row of a `Controller::Mind` hull, or (17b cut M5.2) a seat's
     `MIND PROFILE` landing on a seated or dormant hull; read back to staff, and to the hull's own pilot
     in their report (M5.6)"; `on_chunk`'s rewrite of the mirror's card from it (`arena_bridge.rs:3114`,
     `:3140`, `CardSource::OnTape`, `:628-636`) now also follows a seat's switch, which is the card that
     will fly — said in that doc;
   - `Refusal::Profile` joins `Refusal::as_str` (`:556-580`) and the carried `FrameNotice::Refused`;
     `IntentKind` and `Refusal` gain a variant under format 10 (older documents decode; no build reads
     format 10 without them, since nothing deploys before the gate); the intent's station verb, `MIND
     PROFILE`, lands in M5.4, inside the same cut (D-UM-10's "in the same cut").
5. **The row flies the active profile.** `adopt_mind` (`arena/mind.rs:1453-1490`) laying a NEW row
   lays `MindState::fresh_with(&arena_cfg.reflex, card)`, the card the unit's active profile names
   (`ProfileSet` code at `card_profile.index`); a STANDING row keeps its own card (written by the
   switch; `@mind/set`'s lane unchanged). `schedule_first_mind_thinks` (`:1417-1426`) therefore lays the
   fill's rows on the lead's preset. With `ProfileSet::LINE` and index 0 the card is `card::line(reflex)`
   exactly as `MindState::fresh` lays it (`:816-833`). Like today's Line, the card is derived from the
   arena's own carried doctrine; a test holds the derived card's digest equal to the sealed
   `card_digest` for every preset at the stock config.
6. **The `Withdraw` seed gated on the doctrine bit** (`arena/mind.rs:1468-1471`): seeded only when
   `seal.doctrine`; a non-Doctrine mind is handed its own memory untouched (`seed_withdraw`,
   `:1220-1235`, decodes any memory as `StockMemory` and drops foreign keys). Reachable only with a
   fixture mind (OQ-2).
7. **STOP E's charge** (doc 17 §24 :1805; the owner's 2026-10-09 remedy): `MindState.order_charge:
   Option<ObjId>` (`arena/mind.rs:699-800`), written beside `order` in `apply_intent`'s Order arm from
   `Objective::Support { of }` when `of != unit` (a self-charge is `Engage`, `arena/intents.rs:1386-1391`),
   cleared wherever `order` and `order_issuer` are (a release, an expiry, a possess, the arena's own
   end); `end_mind_support_whose_charge_left` (`arena/mind.rs:2771`) filters on `order_charge` naming
   a unit not in `Arena::units` in place of the supporter's contact row — closing both residuals (a
   `Support` landed `of: None`; the supporter blind for a contact TTL); its doc's "Two residuals"
   paragraph (`:2758-2770`) rewritten. **Folded inside the order's own bytes**: item 3c's `Support` arm
   (`arena/hash.rs:935-937`) folds, after `of`'s `u64`, `order_charge` as `u32` (`0` for none) — so only
   a row holding a `Support` folds a new byte. Not on the view: parity untouched. Latent in production
   (no path removes a unit, `arena/catchup.rs:568`, `:584` are tests).
8. **The `af1` walk** (`tests/arena.rs:5676-5742`): the walked `af1_mind_squad_arena` mover given a
   route of two cells, one avoided cell, a `refused` stop, `searched_from`, a landed `Support` order
   with `max_range_band` and its `order_charge`, a chosen `SealRef.profiles` and a non-default
   `UnitState.card_profile`, so the inner shapes the pin did not walk (doc 17:1808: route and avoid
   elements, `Refusal`, `RangeBand`, the other objectives) and every new field are under it;
   `AF1_SHAPE_DIGEST`/`AF1_SHAPE_LINES` re-blessed once; the limit paragraph (`:5695-5718`) rewritten.
9. **`SNAPSHOT_FORMAT` 9 → 10** (`arena/snapshot.rs:288`), its doc paragraph naming the four carried
   additions (`SealRef.profiles`, `UnitState.card_profile`, `MindState.order_charge`, the two variants)
   and the void at the first reload (`snapshot format 9 is not 10`, AF.3's boot, `:276-283`).
10. **The check-out reads the choice.** `match_::start` (`match_/mod.rs:1120-1133`) builds `SealTerms`
    from `Side.minds` (M5.1): `identity` the held mind's, `profiles` with each card `card::preset(name,
    &arena_cfg.reflex)`, `card` the first, `rated` = `held.rated`, `engine_for_minds` = `held.engine`
    (`PRODUCTION_FILL_ENGINE` for Doctrine; `Scripted` for a fixture); unchosen hulls exactly as at
    `5f505c8`. `open_match`'s mirror cards (`arena_bridge.rs:1683-1697`, `CardSource::CheckOut`) hold
    each hull's sealed first card, not Line.
11. **Q-UM-15** (OQ-5, C-7): `Centre.doctrine_only: bool` (`#[serde(default)]`), written at `SIM START`
    when both sides hold a claimed slot — decided once, from the claims, and printed then; `ready()`
    replaces a non-Doctrine choice under it, `substituted: true`: a seated slot's by Standard Doctrine
    on Line (the fallback preset, constant: no configuration exists, critic G10), and a lead's fill by
    Standard Doctrine **at the lead's chosen profile** when it is one of Doctrine's six presets, else
    Line (doc 17:1488); each sealed with `MindSeal.fallback = Some("humans on the other side")` (the
    field exists unused, `arena/mind.rs:507`). At `SIM START` each pilot whose own choice (or, for a
    lead, whose fill's) is substituted is told, **to that pilot alone** (an `Audience` naming the
    pilot's object; the actor of `SIM START` is whoever typed it), `Humans on the other side: your fill
    and any released hull fly Standard Doctrine.` (80; doc 17:1490-1491). The substitution is taken at
    the seal — the pilot's own mind is not sealed under it (a declared departure, §7 4). Doctrine at any
    preset is never substituted, so **no production match ever says or does this in M5**; `Fixture I`
    reaches it.
12. **The seal line before `STARTUP`** (§12 :947-948, D7, C-18): `run`'s success arm
    (`match_/lobby.rs:2470-2489`), after `join_the_nets` and before `Match N is running. Good hunting.`
    (which stays the check-out's last line, the line every client and wire test waits on), told **to
    each seated pilot whose slot chose a mind or was substituted, to that pilot alone** (an `Audience`
    naming the pilot's object, as the cockpit tells a seated pilot; never `sys_to_actor`, whose actor in
    `run` is the last BOARDer or the deadline's tick, F2): `Sealed for this sortie: Standard Doctrine
    (Sniper, Line), Class I; MIND reads it.` (≤ 90 at the widest preset pair), or with a substitution
    `Sealed for this sortie: Standard Doctrine (Line), Class I, for humans on the other side; MIND reads
    it.` (103); a non-Doctrine seal reads `Sealed for this sortie: Fixture I (Line), Class I; MIND reads
    it.` (the pilot's own mind named to the pilot, C-4). No seal id (C-14). (`MIND` registers in M5.4;
    nothing deploys between them.)
13. **Comments made true in files this sub-cut edits** (F16): `arena/core.rs:163-164` (profiles are 17b
    cut M5's, read at the sealed mind's class); `arena/mind.rs:1170-1171` ("fifteen keys"), `:392`,
    `:441`, `:491`, `:1775` (odds rows: "until the cut that rates a mind above Class I");
    `arena/hash.rs:741-742` (item 3c folds a row for every mind since M4.3); `arena/doctrine.rs:159`,
    `:362`; `match_/seal.rs:21`, `:72`, `:90`, `:99`; `arena_bridge.rs:4861`, `:5359`.

**What moves, and why the note allows it.**
- `SNAPSHOT_FORMAT` 9 → 10 — the one move OQ-1 allows: `tests/arena.rs:5953, 6037, 6177, 6282`,
  `tests/deliberation.rs:2294`, `arena/snapshot.rs`' own test (`:1680`); `every_unit_field` 43 → 44.
- `AF1_SHAPE_DIGEST`/`AF1_SHAPE_LINES` — with the format move and never alone (`tests/arena.rs:5680-5682`).
- The over-class rule's two in-source tests (C-24): `arena/core.rs:645-656` (rated III on the line
  tank's governed Class II core, asserting "the core caps it") becomes a rated II case (`II` on `II`,
  lawful) plus an `Err(SealError::OverClass { rated: III, core: II })` assertion;
  `match_/seal.rs:708-743` (`m4_the_seal_s_class_is_the_least_of_the_rating_the_core_and_the_cap`) keeps
  its lawful rows `(I, III, I)`, `(I, Zero, Zero)`, `(II, II, II)` and the bare (coreless) hull rated III
  → Zero, and its rows `(III, III, II)` and `(III, I, I)` become `OverClass` assertions. Each re-pinned
  with "over-class is refused at the seal since 17b cut M5.2 (doc 17 §9, D-UM-41)".
- The never-folded probe (`arena/hash.rs:2300-2323`) gains `seal.profiles`.
- Hashes: every fixture whose arena holds a mind row with a `Support` order moves once (STOP E's fold),
  each named in the commit body with its old and new values and the reason; the new fixtures. Nothing
  else.
- Wire: the `profile` refusal word — the bot's exhaustive `every_refusal`
  (`crates/mush-bot/tests/arena_contract.rs:453-503`), the TUI's `REFUSAL_WORDS`
  (`crates/mush-net-ssh/src/tui/render.rs:3065`), the web's (`web/src/lib/cockpit.ts:1150`), the line
  client's refusal sentence, composed as every staged refusal is from `as_str` (`format!("{} refused.
  {said}", order.to_uppercase())`, `match_/cockpit.rs:1443-1445`; the web's `${order} refused: …`,
  `cockpit.ts:1225-1236`): **`MIND refused. That profile cannot be switched to now.`**, with no renderer
  special-cased; and `notice_lines.json` (`web/src/lib/fixtures/cockpit/`, hand-written, "together with
  both renderers"); the seal line and Q-UM-15's sentence as neutral-line contract cases.
- Doc comments: `MindCard` (item 4), `continuous_code` (item 4), the seal's (item 1), F16's (item 13).
- Not moved: `m3_start_writes_one_seal_per_unit_and_the_live_record_carries_them`
  (`tests/match_lifecycle.rs:8593`) and the tape-order pins (`tests/arena.rs:6540`,
  `arena_story.rs:1420-1500`) — an unchosen hull's seal and `mind_sealed` line are as before;
  `between_board_and_the_trigger_the_panel_says_it_is_waiting` (`tests/cockpit.rs:850`) — its slots
  choose nothing, so no line enters its window; the `CardUnlawful` pins (`tests/match_lifecycle.rs:8889`,
  `:9866`, red's first pod, armed).

**Why no production hash pin moves** (each claim proved again by running the five before any
re-bless; one that moves is a STOP): (1) every production and fixture seal writes no `profiles` key, so
every `MindSeal::digest`, `seal_digest` fold and `mind_sealed` line is unchanged (F4); (2) `SealRef` is
never folded; (3) `UnitState.card_profile` folds no byte at its default, and nothing in a production pin
switches; (4) `order_charge` folds only inside a `Support` arm, and **no production pin's arena holds a
`Support`**: none of the five issues an order — the Rhai corpus (`M2_CROSS_PLATFORM_PINS`) is played by
`support::play`, which steps with empty intents (`tests/support/mod.rs:1708-1752`,
`arena.step_settled(wall, &[], &[])`; `tests/minds_rhai.rs:3369-3395`), and `m1a5_…`, the duel and the J
field build arenas with no deliberation (`tests/minds.rs:3158-3200`); only the lance lane (not a pin; it
asserts counts, `tests/harness.rs:7414`, `:7711`, ignored) and the deliberation/reflex Z2 fixtures land
`Support`; (5) the row's card is `line(reflex)` for the default set, byte for byte; (6) the seed gate
changes nothing for a Doctrine seal (every arena is sealed through `seal_for` with the Standard Doctrine
identity; the harness's Load III hulls are never released); (7) `IntentKind::Profile` is never issued by
a production pin; (8) the floor probe skipped on an unarmed hull refuses nothing that sealed before
(it only stops refusing). `STANDARD_SHA256`, `MIND_ABI` and `FRAME_VERSION` are untouched.

**Gate evidence.** The three CI-named `--ignored` minds steps in debug as `ci.yml` runs them (the format move; §4.4's commands). `seal.rs` in-source: `m5_a_default_seal_digests_as_it_did_at_5f505c8` (the digest
hex of `MindSeal::fixture` and of the `production_fixture` terms, captured at `5f505c8` before the change
and pinned — written FIRST); `m5_a_sealed_profile_is_named_with_its_card_digest_and_every_card_is_validated`;
`m5_an_over_class_seal_is_refused_by_name` (rated III on the line core; a coreless hull rated III seals
Zero; the heavy rated III seals); `m5_an_unarmed_hull_is_never_refused_for_the_floor` (C-23).
`tests/minds.rs` (the cheap arena rigs): `m5_the_fill_flies_the_leads_preset` (the fill's row card is
Sniper's at its first think); `m5_past_the_grace_a_pilots_hull_flies_its_sealed_profile`;
`m5_a_profile_switch_while_seated_is_flown_at_the_release` and its three-cadence twin
`m5_a_profile_switch_is_one_event_list_and_one_hash_sequence_at_three_cadences` (`step_settled` at
5,000, 1,000 and 500 ms, 17b §0.4); `m5_a_second_switch_inside_ten_seconds_is_refused_profile`;
`m5_a_cold_or_immobile_hull_switches` (a cold tank and a tracked-out tank both land the switch);
`m5_a_switch_and_a_heading_in_one_pulse_both_land` (the code-5 key);
`m5_a_profile_switch_from_a_mind_or_a_slot_is_refused_channel`;
`m5_a_release_hands_a_non_doctrine_mind_its_own_memory` (a `support::scripted` runner on a non-Doctrine
`SealRef`; the seed gated); `m5_a_dormant_rows_card_follows_the_switch`. `tests/deliberation.rs` or
`tests/minds.rs`: `m5_a_support_landed_blind_is_ended_when_its_charge_leaves` and
`m5_a_supporter_blind_for_a_ttl_is_ended_when_its_charge_leaves` (units removed as
`arena/catchup.rs:568` removes them). A resume across a snapshot holding a switched profile and a
chosen set (`af3_snapshot_at_or_after`, `tests/cockpit.rs:12359`) reproducing the hash sequence.
`tests/match_lifecycle.rs`: `m5_check_out_seals_the_slots_choice_and_the_leads_fill` (Doctrine, and
`Fixture I` on a `line_tank` sealed on its own engine with its source found); `m5_the_sim_seal_takes_no_seat_and_posts_nothing`
(the check-out's write list with a chosen mind equals the list without one; no key outside tacsim's
families); `m5_a_range_check_out_seals_its_coreless_target` (`SIM RANGE`'s `target_mk1`, which fits no
core: sealed, never over-class, C-24); `m5_humans_on_both_sides_substitute_a_non_doctrine_choice_and_say_so_once`
(`Fixture I`: the slot's choice sealed as Doctrine on Line, a lead's `Fixture I PROFILE Sniper` fill as
Doctrine at Sniper; the sentence once, to each affected pilot alone; the seal's `fallback`);
`m5_the_seal_line_reaches_the_choosing_pilot_alone_before_good_hunting` (the last BOARDer is NOT the
choosing pilot, so an actor-only write would fail it). The five production pins run unchanged before any
re-bless (their names and values in the commit body). The harness delta, without bless, every block
equal. `cargo test -p mush-bot`. The web lane (`REFUSAL_WORDS`, `notice_lines.json`).

**Risks.** The largest sub-cut: a seal-shape mistake moves every digest — the default-digest pin is
written FIRST, against the `5f505c8` value. STOP E's fold must sit inside the `Support` arm, never as a
per-row presence byte (critic §5.2). The active-profile fold must be conditional on the whole default
(index 0 AND no stamp): a switch back to the first profile still folds. That sub-item writes no byte for
the default where item 1's other sub-items always write one — items 3b to 3e's "no byte for none" rule
applied inside the unit's row — and `state_hash`'s doc (`arena/hash.rs:50-112`) says so. `SealRef` stays
`Copy` only because M5's profiles are preset codes; M6's authored profiles change that.

### M5.3 `tacsim:` release to the sealed mind — the board, the sentences, D14

**Contents.**
1. The board's drop clause `, flown by doctrine` → `, flown by its mind` (`match_/lobby.rs:3906-3914`);
   `, left` kept for an `EXIT` (C-11); one word for every mind, so nothing leaks (D11).
2. The reconnect line (`match_/resume.rs:77-83`, `doctrine_line` → `mind_line`): `Red Tank 1 is flying
   on its mind; BOARD takes it back.` (54); said by `SeatFacts::connected` and the boarding tick as
   now.
3. The release acknowledgement (`match_/cockpit.rs:7909-7915`, its doc example `:7887-7888`):
   `Released red-1 to its mind: it fights on its own until it is ordered again.` and `Released red-1
   and red-2 to their minds: they fight on their own until they are ordered again.` — "to doctrine"
   becomes false under a lead's `SIM AI MIND` and names a mind (carries map §3.2).
4. **D14** (doc 17:1805): `"unknown"` → `flying on its own mind` in line mode (`machine_state_words`,
   `match_/cockpit.rs:1484-1494`), the TUI (`crates/mush-net-ssh/src/tui/render.rs:3383-3392`) and the
   web (`web/src/lib/cockpit.ts:1330-1337`, `MACHINE_STATE_WORDS`) — `  The machine was flying on its
   own mind under no order.` — and `catchup_lines.json`'s row (`web/src/lib/fixtures/cockpit/`,
   hand-written, both renderers; also read by `match_/cockpit.rs:8480` and
   `crates/mush-session/tests/resume.rs:1046`); the bot reads it as `Line::Other`, unchanged
   (`lines.rs:2739-2740`). Reachable only with a fixture mind.
5. The sortie audit (C-11a): no other cockpit sentence changes.
6. Docs and comments: `Slot::is_dropped` (`match_/lobby.rs:148-155`), `:151`, `:2204`, `:3790`,
   `arena_bridge.rs:3537`, `crates/mush-config/src/config.rs:1342`; `docs/help/simulator.md:311`,
   `:330`; `docs/help/cockpit.md:480-481`; `docs/bot.md:429`.
7. **The bot** (declared reader change, critic G4): `board_pilot` reads `, flown by its mind` as the
   resume standing (`lines.rs:1040-1044`; `Standing::Doctrine` renamed `Standing::Mind`), the reconnect
   reader (`:1834`), and the unit cases (`lines.rs:2749`, `:3076`, `:3095`;
   `crates/mush-bot/src/arena/lobby.rs:2451`, `:2545`; `crates/mush-bot/src/arena/picture.rs:3782`,
   `:3884`).

**What moves.** The 27 sites of the old word (critic G4: `match_/lobby.rs:3907` and its docs;
`tests/cockpit.rs:1983, 7559, 12600, 16440, 16677`; `crates/mushd/tests/sim_cockpit.rs:900`;
`crates/mushd/tests/sim_reflex.rs:539`; `crates/mushd/tests/observer_rule.rs:3936, 4753`;
`crates/mushd/tests/support/mod.rs:59-71`); the reconnect pins (`sim_cockpit.rs:903`,
`crates/mush-gateway/tests/arena_custody_split.rs:863`, the bot's); the release pins
(`match_/cockpit.rs:10381, 10392, 10403`; `tests/cockpit.rs:8521, 8660, 8738, 8773`;
`crates/mushd/tests/sim_solo.rs:1010`); `catchup_lines.json`. Sentences only — the scope's
deliverable 7 and the D14 carry. **No hash, no format, no frame.**

**Gate evidence.** `cargo test -p mush-bot`, with reader cases for the new standing and the new
reconnect line, and the resume flow asserted end to end in `arena_lane` (the bot reads its row `flown by
its mind`, follows the match and boards back); the gateway twins (`arena_custody_split`);
`sim_cockpit::past_the_grace_the_machine_flies_the_tank_until_the_pilot_boards_again` (`:847-990`) on the
new words; `m5_d14_the_catch_up_says_its_own_mind_in_three_renderers` (line mode in-source; the TUI's
`catchup_words` pairs, `render.rs:6261`; vitest); `cargo test -p mush-session` (the resume test reading
`catchup_lines.json`); the web lane (`MACHINE_STATE_WORDS`, `catchup_lines.json`).

**Risks.** The bot's resume (G4): an old bot binary against the new server reads the row as a name and
never resumes — deploy them together (§5). Any word that differs between Doctrine and another mind
leaks the choice; there is one.

### M5.4 `tacsim:` the `MIND` verb in line mode, its keyframe object, and `MIND PROFILE`

**Contents.**
1. **The seventeenth cockpit verb.** `"mind"` appended to `COCKPIT_VERBS` (`match_/cockpit.rs:205-222`,
   `[&str; 17]`), as `drive` was; its `CockpitGate` arm (`:8001-8070`: the deployed unit's station
   only); `cockpit_help("mind")` (`register`, `:8285-8345`); the module head's `//! MIND` row (`:11`,
   `:94`, read by `d2_the_verb_table_and_the_help_agree_with_the_selector_list`, `:9182-9200`); a
   non-baseline `mind` stanza in `legacy-handlers.toml` declaring the prefix shift (item 7).
2. **The data: `Frame.mind`** (`arena/step.rs:88`), built by `Arena::frame_for` (`arena/step.rs:2310`,
   under `frames`, `:2258`) for the frame's own unit — `FrameMind { doctrine: bool, profiles:
   ProfileSet, active: u8, class: CoreClass, core_class: CoreClass, switch_at_ms: Option<i64>, off:
   bool }`, `PartialEq`, off the unit's `SealRef` (`doctrine`, `profiles`), the classes **read live**
   — `class` = `Arena::mind_class(unit)` (`arena/core.rs:432-457`: the sealed class against the core
   NOW) and `core_class` = `Arena::core_of(unit)`'s class (`read_core`, live, `:425-430`), never
   `SealRef`'s figures fixed at the seal, so a heavy whose core a `degrade_core` has lowered reads the
   Class II it now is (the seal line, said once at check-out, keeps the sealed figures) — its
   `UnitState.card_profile` (`active`; `switch_at_ms` = `switched_ms + 10,000` whenever `switched_ms` is
   set — absolute, so it changes only at a switch) and its mind row if one stands (`off` = the row is
   dropped to the ROM); `None` on a hull with no seal (`SealRef::none()`). **`Frame::same_as`**
   (`arena/step.rs:419-489`) gains `self.mind == other.mind` with its one-line reason, in the function's
   own style ("a switch, the next-switch moment, a degraded core and a drop to the ROM are what `MIND`
   prints; the sealed set is a per-match constant and compares equal") — without it a switch on a quiet,
   stationary hull sends no frame and every reader of the held frame reads the old profile. **Never
   copied into `MindView`** (`Arena::mind_view`, `arena/mind.rs:1794`): a tier truncation (D-UM-4), the
   114-row `PARITY` table untouched.
3. **The keyframe key `mind`** (`status_frame_held`, `match_/cockpit.rs:5851`), on a seated pilot's
   status frame only (the replay builds its window from the tape and the watch's frames carry no unit
   of the reader's, so neither carries the key): `{"name": "Standard Doctrine", "profiles": ["Sniper",
   "Line"], "active": 0, "class": "I", "core": "II", "channels": ["helm", "sensors", "fire"],
   "next_switch_ms": null, "off": false}` — `name` is `Standard Doctrine` for a doctrine seal and `null`
   for any other (the arena keeps no mind name; every renderer prints `your own mind`; fixture-only in
   M5, and the cut that seals a non-Doctrine mind in production carries its name as a key added for a
   person); `next_switch_ms` the absolute arena millisecond of `switch_at_ms`, `null` before any switch;
   `channels` the stations the pilot holds (`fire` on an armed hull, the frame's own rule,
   `:5857-5860`). **`FRAME_VERSION` stays 3** (F7; the policy's text at `:5662-5677` gains one
   sentence: a field added for the person in the seat is not a version); consumers ignore unknown keys
   (`web/src/lib/cockpit.ts:841-897`), so the TUI and the web draw nothing of it until M5.5.
4. **Bare `MIND`** — answered world-side from the held keyframe (`say_and_frame`, `:5636-5653`, the
   `WEAPONSTATUS` model `:5298`), at most seven lines, each under 120 columns (measured at the widest
   preset pair); any time it prints is read against **the mirror's arena clock** (`MatchMirror.arena_ms`,
   `arena_bridge.rs:527`), never the held frame's `t_ms`, which a quiet frame leaves stale:
   - `Mind: Standard Doctrine, sealed for this sortie; it thinks at Class I on a Class II core.` (89;
     a non-Doctrine seal reads `Mind: your own mind, …`; both classes are the frame's live ones, item 2,
     so a degraded core reads as it is now)
   - `Profile: Sniper, 1 of 2 (also Line); MIND PROFILE Line switches, once in ten seconds.` (≤ 99)
   - `Channels: you hold helm, sensors and fire; the mind is dormant while you fly.` (77) — on an
     unarmed hull `Channels: you hold helm and sensors (no weapons station on this hull); the mind is
     dormant while you fly.` (105; §11 :864-866), the block's line in the weapons block's place (§22
     italic);
   - `Rules: none sealed; your own armings are under WEAPONSTATUS.` (60)
   - dropped: line 1 reads `Mind: Standard Doctrine, off for the match: the hull is on the ROM.`
     (`ROM_TAIL`'s words, `arena_bridge.rs:4854`); no seal: `This hull carries no seal.`
   The think lines read "dormant while you fly" (OQ-8); no ops figure (F8); no seal id (C-14).
5. **`MIND PROFILE <name>`**: the cockpit pre-checks off the held keyframe and the mirror's clock, each
   refusal one line, every one opening `MIND refused.` as the arena's own word does (item 4 of M5.2), so
   a pilot reads one prefix for a refused switch whichever side refuses it — `MIND refused. Hunter is not
   sealed on this mind; it carries Sniper and Line.` (≤ 85), `MIND refused. Sniper is already the
   profile.` (44; the arena refuses the same case `profile`, so it opens the same way), `MIND refused.
   One switch in ten seconds; the next in 6 s.` (≤ 58; N from `next_switch_ms` against
   `MatchMirror.arena_ms`) — no power test (M5.2 item 4: a cold hull switches) — then stages
   `IntentKind::Profile { index }` through the seat on helm and says `Profile Sniper: the mind will fly
   it when you leave the seat.` (≤ 65); the arena's own refusal, if one lands, reads `MIND refused. That
   profile cannot be switched to now.` (53). **The specific-versus-generic split is `NoJammer`'s
   precedent, declared** (`arena/intents.rs:526-531`; 16b cut B-16's one-string rule,
   `match_/cockpit.rs:1425-1436`, holds for the PREFIX — every refused switch opens `MIND refused.` —
   while the cockpit, which reads the frame, says which of the four cases it is and the arena, reached
   only by a race the frame did not see, says one generic sentence for all four); the composition is
   `order_refused`'s `{AS_STR} refused. …` with `as_str` `mind` (M5.2 item 4), and the local pre-checks
   build the same prefix through the same function.
6. **`MIND RULES`**: `No rules are sealed on Sniper or Line; your own armings are under WEAPONSTATUS.`
   (≤ 88). **`MIND TRACE [n]`**: refused in one sentence until M6's ring (OQ-8): `MIND TRACE refused.
   This core keeps no trace of its thinks yet.` (63; a form refused, not an intent, so it names its
   form).
7. **The prefix shift — resolved, no STOP.** `lookup_idx` (`crates/mush-core/src/cmd.rs:1095-1147`)
   tries exact names and aliases first, then prefixes of bare PRIMARY names only; `@`/`+` names and
   aliases never prefix-match. The production roster (`legacy-handlers.toml`) has no bare primary verb
   starting with `m` (its m-names are `+map +market +move +muster @macro @mail @market @mind`; `move` is
   only `goto`'s alias, `crates/mush-core/src/commands/movement.rs:293-298`). So `m`, `mi` and `min` go
   from `NotFound` to `mind` (the `CockpitGate`'s one line outside a pod), and no kernel verb loses a
   prefix. Pinned by `m5_mind_collides_with_no_registered_command` (`m`, `mi`, `min`, `mind` →
   `Ok("mind")`) and declared in the stanza, as `dr` became `Ambiguous(drive, drop)` (doc 17:1754).
8. **The bot**: `MIND` joins `arena_e2e.rs`' `forbidden`; a neutral-line contract case for each `MIND`
   line and each `MIND PROFILE`/`MIND TRACE` sentence (the `the_drive_notices_read_as_neutral_lines…`
   template, `crates/mush-bot/tests/arena_contract.rs:2055-2077`).
9. **Comments made true or re-pointed** (F16): `arena/step.rs:320` (odds rows: "until the cut that
   rates a mind above Class I"); `match_/verbs.rs:1287` (the pilot's `MIND` verb is built);
   `match_/cockpit.rs:633`, `:4869`, `:4902` and `arena/conditions.rs:53`, `:146` ("M5's `LOCK`
   rules", missed by the draft's grep), `:204` (re-pointed: "the rule-language cut after 17b cut M5").

**What moves, and why the note allows it.** `COCKPIT_VERBS` 16 → 17 (a reviewed diff doc 17 §19
names): `match_/cockpit.rs:8848, 9183, 10528, 10583-10584` (`m1a4_drive_is_the_sixteenth_selector…`
keeps `get(15) == "drive"` and the length becomes 17), the head `:11`, `:94` ("Nineteen … seventeen"),
`tests/cockpit.rs:802, 869, 13970` (they iterate), `docs/help/cockpit.md:3` ("The seventeen verbs"),
the `drive` stanza's "eighteen verbs with BOARD and EXIT" (`legacy-handlers.toml:771-773`); the
canonical `cockpit_frame.json` **and `view_frame.json`** (which embeds every cockpit keyframe key beside
the grid's) gain the `mind` key with `"version": 3` unchanged (`TACSIM_BLESS=1 cargo test -p tacsim
--test cockpit`; the fixture README says the key was added without a version); `web_view_model.txt`
does NOT move here (the decoder ignores the key until M5.5; the web lane proves it); `Frame::same_as`'s
doc; the bot's contract. Hashes: only the fixtures that switch. No format (M5.2 carried everything), no
seal, no `MIND_ABI`.

**Why no production hash pin moves.** `Frame` is output, never hashed or carried (F7); a switch is
folded only where one happens (M5.2's sub-item 20), and no production pin types `MIND PROFILE`.

**Gate evidence.** `match_/cockpit.rs` in-source: `m5_mind_is_the_seventeenth_selector_and_its_words_are_pinned`;
`m5_the_mind_block_is_at_most_seven_lines_under_120_columns` (the widest preset pair, an unarmed hull, a
dropped row, no seal, a non-Doctrine seal); `m5_mind_profile_refuses_off_the_frame_in_one_line` (every
pre-check, the countdown against the mirror's clock). `arena/step.rs` in-source:
`m5_a_switch_alone_changes_the_frame` (`same_as` false across a switch on an unchanged pose);
`m5_the_frames_mind_reads_the_core_class_now` (a heavy rated and sealed at Class III under a raised
`fill_class`, its core degraded to II by `degrade_core`: `FrameMind.core_class` II, `class` II, and
`same_as` false across the degrade).
`tests/cockpit.rs` (the production roster rig): `m5_mind_collides_with_no_registered_command`;
`m5_mind_profile_switches_a_sealed_profile_at_most_once_in_ten_seconds` (the seat, the frame's
`next_switch_ms`, the `mind_card` line, a release flying the switched card, a cold tank's switch);
`m5_a_switch_on_a_stationary_hull_sends_one_frame_and_mind_reads_it` (a parked, quiet hull: exactly one
keyframe after the switch, its `mind.active` moved, and `MIND` printing the new profile);
`m5_a_staff_read_back_follows_a_pilots_switch` (a seated pilot switches to Line, then `EXIT`s: the
switch's `mind_card` line, absorbed by `note_cards_from_tape` like a staff edit's
(`arena_bridge.rs:3105-3150`, `CardSource::OnTape`), leaves the world's mirror on Line, so staff's bare
`@mind/set <unit>` reads Line back and a staff `@mind/set` after the release computes `changed` off
Line, not off the first sealed card `open_match` laid, `CardSource::CheckOut`);
`the_canonical_cockpit_fixtures_are_what_the_builders_produce` (`:2933`) re-blessed;
`crates/mushd/tests/observer_rule.rs`: `m5_the_mind_object_names_nothing_but_the_pilots_own_seal` (the
`m1b3_the_arming_names_nothing_but_the_pilots_own_slot` pattern, `:7641`: Ada's frame over WS carries her
`mind` with `Sniper`; Bo's carries his own and no byte of hers). `cargo test -p mush-bot`. The harness
delta, without bless, every block equal. **The fan-out lane's keyframe bound, re-read**: the `mind`
object adds about 200 bytes to every seated pilot's keyframe, and `tacsim_harness_fan_out` (24
staff-seated pilots, `tests/harness.rs:6737-6743`) bounds `panel_bytes_max` by
`[tacsim.harness].max_frame_bytes` (16,384, `config.rs:1224`) — a figure outside the deterministic
block, so the delta never reads it; its `harness_fan_out:` line (`tests/harness.rs:7168`) from the
release lanes' run is quoted on M5.3's tree and on M5.4's, `panel_bytes_max` and
`events_per_pulse_max` before and after, each under its bound (`TACSIM_ASSERT=1 TACSIM_JSON=harness.json
cargo test --release -p tacsim --test harness tacsim_harness_fan_out -- --ignored --exact --nocapture`
asserts both). The web lane (the two re-blessed fixtures read, the view model unchanged).

**Risks.** The new intent's place among `apply_intent`'s authority checks (M5.2 item 4 settles it: its
own arm before the continuous gates). A switch typed while no row stands is read at the release (M5.2's
adoption) — the evidence above covers both orders. `same_as` grows by one comparison per frame (the
harness delta reads it).

### M5.5 `mush-net-ssh`/`web`: the MIND block on the TUI and the web — the weapons pane's tail

**Contents.**
1. **TUI** (`crates/mush-net-ssh/src/tui/`): **the MIND block is the weapons pane's tail** — four rows
   drawn under the weapon rows inside the existing `weapons` pane (`render_cockpit_weapons_rows`,
   `render.rs:2781`, gains `render_cockpit_mind_tail`), and on an unarmed hull (no `weapons` key) the
   same four rows replace `(no weapon)` (`render.rs:2793-2795`) — "in its place" with no mechanism to
   add (`PaneSpec` has no visibility predicate, `tui/screen.rs:205-222`). Rows, each at most 44 columns
   inside the 46-column Right column at 120:
   `mind Standard Doctrine · Class I` (or `mind: your own · Class I`) / `> Sniper   Line` (the active
   profile marked) / `dormant while you fly` (or `off for the match: on the ROM`) / `you: helm sensors
   fire`. No countdown is drawn (the keyframe's `next_switch_ms` is absolute, and a quiet frame would
   leave a drawn countdown stale; line mode's `MIND` prints it off the mirror's clock). **No pane is
   added and no split changes**: the Right column stays `weapons`, `contacts`, `notices` on its even
   `Ratio` split (`tui/app.rs:3364-3424`), `COCKPIT_ROWS` stays 30, `degrade` stays `[damage, weapons,
   contacts]` (`screen.rs:1149`), so 16b cut AB.1's contact budget is untouched — a fourth Right pane
   would cost the contacts pane two rows at 120×34 (X4 below the fold) and at 120×30 (X3), the budget
   test's content assertions (`tests/tui_golden.rs:925-1003`). At 120×34 the weapons pane holds seven
   inner rows, three of them the canonical frame's weapons and arming line, so the tail is whole; at
   120×30 it holds six, so the tail's fourth row is clipped from the bottom (a non-selectable panel
   scrolls at 0, `tui/app.rs:3521-3548`) and the weapon rows are never displaced — the rows are ordered
   so the channel row, the least needed in the weapons pane, goes first. No key is bound
   (`run-allowlist.json` unchanged). The replay (`SIM REPLAY`) and `@arena/watch` screens, which paint
   the cockpit's panes (`tui_golden.rs:1714-1720`, `:1874-1879`), draw no tail: their payloads carry no
   `mind` key (M5.4 item 3).
2. **Web** (`web/src/`): `CockpitFrame.mind` decoded in `lib/cockpit.ts` (`decodeCockpitFrame`,
   `:841-897`) with `mindLines` beside `weaponLines` (`:1618-1628`); the lines drawn as the weapons
   section's tail in `components/CockpitScreen.svelte` (`:293-298`), in its place on an unarmed hull —
   the same placement as the TUI, so the section headings stay five; a profile row's click calls
   `prefill("MIND PROFILE <name>")` and sends nothing until Enter (`:104-108`; C-6).
3. Both read the same keyframe object M5.4 publishes; line mode, the TUI and the web print the same
   facts. The card's slider drawing and the rule glyphs are M6's (C-22).

**What moves.** The four `tui_cockpit_*` goldens (`crates/mush-net-ssh/tests/golden/tui_cockpit_120x34.txt`,
`_jammed`, `_catchup`, `tui_cockpit_160x48.txt`: the weapons pane gains its tail; `UPDATE_GOLDEN=1 cargo
test -p mush-net-ssh --test tui_golden`); `the_cockpit_screen_paints_at_both_budget_sizes`
(`tui_golden.rs:936-1053`) gains the tail's assertions — its contact assertions and their doc are
unchanged; the five pane titles (`:952-957`), the replay's and the watch's (`:1715`, `:1874`) and the
floor test (`:347`) unchanged; `web_view_model.txt` (`COCKPIT_BLESS=1 npx vitest run
src/lib/cockpit.test.ts` from `web/`); `web/src/components/CockpitScreen.test.ts` gains a tail case and
a prefill case on the `:184-195` model (its five headings, `:174-182`, unchanged);
`tests/cockpit.rs:14301-14312` re-reads the two goldens it includes. Renderers only — the scope's
deliverable 4. **No hash, no format, no frame version, no sentence.**

**Gate evidence.** The bless commands above, each run once and the diff read; the web lane (`npm ci`,
`npm test`, `npm run build`; the `web` CI job, `.github/workflows/ci.yml:593-612`);
`the_cockpit_screen_paints_at_both_budget_sizes` with the tail whole at 120×34, its first three rows
whole at 120×30, and the four contacts (120×34) and three (120×30) whole as before;
`m5_the_mind_tail_takes_the_weapons_place_on_an_unarmed_hull` (TUI and vitest, the drone);
`m5_the_replay_and_watch_screens_draw_no_mind_tail`; `the_cockpit_screen_reads_the_frame_and_nothing_else`
(`tui_golden.rs:1357`) unchanged in meaning. The `truth-serialization-policy` job's
`view_rows_reproduce_actions_and_never_leak_undeclared_fields` (`render.rs:4380-4405`) builds its own
`actions` view and reads no cockpit frame; nothing is declared there.

**Risks.** The web lane's 569 tests. A canonical frame with more weapon rows than three would push the
tail below 120×34's fold; the budget test pins the canonical frame's.

### M5.6 `tacsim:` `SIM REPORT`'s `Your mind` block, the first hint, `help minds`, the help made true

**Contents.**
1. **The `Your mind` block** (OQ-9 (ii)) — `Report.mind: Option<MindBlock>`
   (`match_/report.rs:518-537`, `#[serde(default, skip_serializing_if = "Option::is_none")]` on the
   report document, `report_document` `:1674`; the live `Fold`'s new fields `#[serde(default)]` under
   `LIVE_FORMAT`'s policy).
   - **Its check-out facts on `with_own_tank`'s precedent** (F12; not a departure): `match_::start`
     seeds each seated pilot's fold beside `.with_own_tank(…)` (`match_/mod.rs:1384`) with
     `Fold::with_own_mind(OwnMind { chose: bool, mind: SmolStr, profiles: Vec<SmolStr>, substituted:
     bool })` from `Side.minds` — whether the slot chose (a chosen `SIM MIND Standard` seals
     byte-identically to an unchosen slot, F4, so the tape cannot say it), the mind and the profile
     names as sealed. No tape line changes shape.
   - From the tape, in the pilot fold (`absorb_pilot`, `:663`), on the pilot's OWN hull: `mind_card`
     (a switch, printed by the card's own name — a staff `@mind/set` on the hull appears the same way,
     `Sniper, edited (min_odds 70)`, and `help minds` says so, D14), `mind_dropped` (the cause word
     only, D17) and `fired`/`hit`/`unit_destroyed` for the shot count.
   - **When it flew** is the one fact neither the tape nor the check-out carries (F12):
     `lobby::slot_left` (`match_/lobby.rs:2232`, the funnel every `Released` passes — `EXIT`, `QUIT`, the
     grace, a resume) opens a span on the pilot's fold at the world's arena clock
     (`MatchMirror.arena_ms`, `arena_bridge.rs:523-527`), and `slot_reclaimed` (`:2347`) closes it; a
     declared departure from 16b cut V.3's "told from the tape" for this one line (§7 3).
   - **Its shots**: the hull's `fired` lines whose `t_ms` lies inside a span, with their hits and kills.
     The span's edges are the mirror's clock, so a shot in the pulse of a hand-over may be counted on
     either side of it (declared with §7 3). **The pilot's own lines are untouched**: `Report.rounds` and
     `kills` (`:531-534`), the `You fired …` line, the timeline and the report document's counts keep
     counting the hull's every shot exactly as at `5f505c8`; no timeline line is prefixed or moved; the
     block alone is new.
   - Printed only when `OwnMind.chose` or a span opened (OQ-9); every line under 120 columns and free of
     `#`, `seed`, `hash`, `draw`, `damage`, `" m:"` and sixteen hex (`tests/report.rs:46-63`;
     `observer_rule.rs::report_is_the_pilots_own`, `:3017-3031`):
     - `Your mind: Standard Doctrine (Sniper, Line); switched to Line at 1:05.` (≤ 85)
     - one span: `  It flew your tank from 0:42, when you climbed out, to the end.` (64), or `  It flew
       your tank from 0:42 to 1:10, while your line was down.` (`… to the end, …` when never reclaimed);
       two: `  It flew your tank from 0:42 to 1:10 and from 2:05 to the end, while your line was down.`
       (89); **three or more, bounded whatever the count**: `  It flew your tank 5 times: first from
       0:42 to 1:10, last from 12:05 to the end.` (≤ 86 at three-digit counts and `59:59` times; no
       cause clause, since the spans' causes may differ); or `  It never flew: you flew the tank the
       whole match.`
     - `  Its shots: cannon 4, 2 of them hits; it destroyed nothing.` (60)
     - `  It dropped to the ROM at 1:30: its core was destroyed.` (56)
   The staff fold is unchanged (`:1285-1326`).
2. **The first hint** (doc 17 §15 :1124-1126, reworded by C-12): `HintPoint::Mind`, the sixth
   (`match_/tutorial.rs:107-128`), said by `match_::finish` (`match_/mod.rs:1743`) once per account to
   each human pilot of the match still connected (`tutorial::hint` needs live connections, `:342-345`):
   `Hint: SIM MIND Standard lets your tank's core fly the Registry's doctrine your way when you climb
   out.` (102 columns, no digit, no `Tank`, `#`, `X1`, `BA` or `RA`). The second hint is M6's (OQ-10 (a)).
3. **`docs/help/minds.md`** (new; bundled into the bot by `crates/mush-bot/build.rs`), opening with
   three lines pinned true (doc 17:1740-1742) — the draft's first said `SIM MIND` "seals" (it chooses;
   the seal is taken at check-out, C-2) and its third that the mind flies when "your line drops, until
   you board again" (a drop hands the tank over only past the grace, and after an `EXIT` nobody boards
   again, `match_/lobby.rs:89-101`):
   - `Your tank's core can fly it when you are not in the seat. SIM MIND chooses its mind; the match's
     start seals it.` (112)
   - `With none chosen, a tank you leave flies Standard Doctrine on Line, the Registry's own; MIND reads
     it in the cockpit.` (117)
   - `It flies from when you climb out to the end, or from a drop past the grace until the end or you
     board again.` (108; "board again" belongs to the drop's clause alone)
   then the choice and its fit (the class, the profile count, the floor on an armed hull, the pool
   sentence), the six presets with the terms each one changes that the doctrine flies and the posture
   terms it carries inert (`stance`, `opening`, `stop_unseen` and the rest of the eleven `[M2]` sliders,
   OQ-10 (e)), `MIND PROFILE`'s ten seconds, the fog (who reads your choice, §2.4), that an operator's
   change to your hull's card shows in your report by its name, and what arrives later (no promise of a
   verb that does not exist: no `@mind`, no `HOLD FIRE`). Every command-shaped word resolves or is
   allowlisted (`crates/mush-contract-tests/tests/help_commands.rs:184`): `PROFILE` appears only inside
   code spans that open with `SIM` or `MIND`.
4. **The help made true** (OQ-6, the text in M5; the behaviour in the residues cut):
   - `docs/help/simulator.md:202-205` (STOP A): the machines do not drive for the key spot; they fight
     what they find, and the spot is the pilots' to take;
   - `docs/help/cockpit.md:477-479`, `:497-505` (STOP D, and the support-end sentence): a supporter
     keeps station on its charge and fights what it finds there; if the charge leaves the match the
     order ends for them (true for every `Support` once M5.2 lands), and a supporter that sees it
     destroyed fights on its own;
   - `order_ack`'s two `Support` arms (`match_/cockpit.rs:7961-7974`, doc `:7885-7886`): `Ordered
     red-1 to support RA: it keeps within 150 m of RA and fights what it finds there.` and `…, holding
     fire: it keeps within 150 m of RA and locks what it finds but fires nothing.`;
   - `docs/help/cockpit.md:445-446` ("The rest of the rule language arrives with the mind's own card")
     → "The rest of the rule language comes in a later cut." (no profile carries rules in M5);
   - the drone's note (`crates/tacsim/content/units/drone_mk1.json:90`, "in a Hold match drives to the
     spot but holds nothing") made true, its manifest digest recomputed (`:6`) and its version 2 → 3
     (`:4`);
   - the leader's system prompt (`deliberation/request.rs:539-548`): support "keeps station on one of
     your own members and fights what it finds there"; the key-spot sentence says the squad does not
     drive for the spot unless ordered to advance on it; "own reflexes" → "its own doctrine" (`:541`,
     `deliberation/squad.rs:61`);
   - `docs/help/cockpit.md` and `simulator.md` gain the `MIND` verb, `SIM MIND`/`SIM AI MIND`, the
     board's words and the report's block (`simulator.md:377-391`).
5. **Comments re-pointed** (F16): `tests/extensions.rs:10606` (STOP I: "carried to the residues cut
   after 17b cut M5").

**What moves, and why the note allows it.** `match_/report.rs:3101` (`pilot.lines.is_empty()` in
`m3_the_staff_read_the_four_mind_lines_and_the_pilot_reads_nothing_of_them`) — inverted for the own hull
(the scope's deliverable 8); every existing report pin over a released hull or a choosing pilot gains
the block (the run names each — expected among `tests/report.rs`' released-hull cases and the mushd
report reads — each re-pinned old → new with OQ-9's reason; none of their other lines moves);
`HintPoint::ALL`/`HINT_LINES` 5 → 6 (`match_/tutorial.rs:122`, `:178`), `read_line`'s "of 5"
(`:248-255`), the character counts (`:532`), `help_commands.rs:368` (`checked == 10` → 12: `SIM` and the
`mind` switch; `help_commands.rs:335-379`), the "of 5" reads (`tests/match_lifecycle.rs:8332-8475`,
`tests/cockpit.rs:13057, 13086, 13156`, `crates/mushd/tests/sim_solo.rs:1251, 1321, 1350, 1272-1281`,
`crates/mushd/tests/support/mod.rs:1061`'s "none of the five"); the `SUPPORT` pins
(`match_/cockpit.rs:10347`, `:10373`; `tests/cockpit.rs:8517`, `:8563`; `sim_solo.rs:954`); the prompt
pins (`deliberation/request.rs:1269-1272`, `tests/deliberation.rs:5306`); the drone pack's two pins —
its version (`tests/content.rs:1268`, drone 2 → 3) and **its digest** (`tests/extensions.rs:10788`,
`DIGESTS_DRONE`, re-pinned with "Was `1bfeca3a…` at version 2"). `match_started` carries no payload and
the snapshot's definition digest is the ground's, so no hash moves. Sentences, help and one content
note — deliverables 8 to 10 and OQ-6. **No hash** (the prompt is not hashed state; no harness pins its
bytes — the run confirms), **no format, no frame.**

**Gate evidence.** `match_/report.rs` in-source and `tests/report.rs`:
`m5_the_pilots_report_carries_their_own_mind_block_and_no_other` (swept by `leak` and
`report_is_the_pilots_own`; Bo's and the bystander's reads carry none of it);
`m5_the_block_is_printed_only_for_a_chosen_or_released_hull` (a chosen `SIM MIND Standard` with no
`PROFILE`, never released, prints it; an unchosen, never-released hull does not);
`m5_the_minds_shots_are_counted_in_its_block_and_the_pilots_lines_are_unchanged` (the pilot's `rounds`,
`kills` and `You fired` equal to the same match's at `5f505c8`); `m5_a_staff_edit_reads_by_its_card_name`;
`m5_the_span_line_is_bounded_at_five_spans` (five drop-and-reclaim spans: one line, under 120 columns,
`5 times: first … last …`).
`match_/cockpit.rs`: `m5_help_minds_opens_with_three_true_lines` (the `ap3_watch_lines_…`
`include_str!` pattern, `:10925-10950`). `match_/tutorial.rs`: the `ah1_*` tests on six lines.
`sim_solo.rs`'s `ah1_a_first_account_reads_five_hints_…` → six, the sixth said after the first finished
match. `help_commands`. `cargo test -p mush-bot` (the help bundle; neutral lines for the new acks).

**Risks.** The hint count pins spread over five files; the span's world clock lagging the arena by up to
one pulse (declared, printed to the second).

### M5.7 `tacsim:`/docs: the gate and the record

**Contents.** The gate transcript, the fog battery and the sweep (§4, exactly); the harness lines
verbatim once (17b §0.5: M5 adds a per-unit branch to the hash and one `Frame.mind` per seated pilot's
frame, compared in `same_as`), the deterministic block equal to the baseline; the F16 sweep's last
sites, in files no earlier sub-cut edits — `arena/damage.rs:1317`, `:1588` (STOP G); `arena/motor.rs:491`
(STOP F); `arena/stock_mind.rs:93` (STOP A); `tests/reflex.rs:50`, `:610` (STOP G), `:9196`, `:10564`
(STOP A); `tests/shipped_scale.rs:21` (STOP K); `tests/harness.rs:7734` (re-pointed: "carried to the
residues cut after 17b cut M5"; `stock_mind.rs:93` is a `//!` comment, not the `.rhai` header, so no
digest moves) — and **the F16 grep** (`rg -n "cut M5\b|until M5|before M5|M5's|to M5\b|M5 admits"
crates web/src docs/help`) run on the merged tree, every remaining hit a true sentence, its output
recorded in the Evidence cell; then the record (§8.1).

**What moves.** The transcript and the fog tests are new; `observer_rule.rs` gains the `boot_filled`
variant (§4.3); comments; nothing else.

**Risks.** The transcript's wall time under 30 s so it stays un-ignored (`ci.yml:52-57`,
`ci_gated_test_inventory`; the M1b transcript ran 26.84 s); the `observer_rule.rs` TUI paint flake
(17b:110, :112).

---

## 4. M5's gate, made checkable (doc 17 §22 :1590-1594)

Run at M5.7 on the merged tree after `git fetch`; every item below is a named test or a command, and
its result goes into 17b's M5 Evidence cell verbatim.

### 4.1 (a) The line-mode transcript on the local rig

**The rig.** "The local rig" is the all-in-one `mushd` (16c:95), booted in-process over real SSH by
`crates/mushd/tests/sim_solo.rs` — the lone pilot against the AI fill (`cfg.match_.ai_fill = true`,
`:137`), the field Q-UM-15 names as unaffected (doc 17:1493-1494) — with the rig's own
`unattended_ms = 10_000` (`:141`), so the match ends itself ten seconds after its last pilot leaves
(`d3_the_match_a_lone_pilot_walks_out_of_ends_itself`, `:766-863`) and the report is written at its
check-in; `startup_ms` 300, so a mind's startup lands well inside the ten seconds. The test,
`sim_solo.rs::m5_a_pilot_seals_a_mind_climbs_out_and_the_tank_is_flown_by_it`, logs Ada in at 120 × 40
(`play_client_sized`, `crates/mushd/tests/support/mod.rs:392`; the `login_wide` shape of the M1b
transcript, `sim_weapons.rs:811-823`), with God logged in for the staff reads, and prints the terminals
between `==== Ada ====` and `==== end (N s) ====` as the M1b transcript does (`sim_weapons.rs:1076-1087`).

**The steps, in order, each asserted on the terminal:**
1. `SIM TEAM red`.
2. `SIM MIND` → `Yours: none; a tank you leave flies Standard Doctrine on Line.`, the `Fits line_tank
   (II): Standard Doctrine, up to two of …` line and the pool line.
3. `SIM MIND Sentinel` → `You hold no mind called Sentinel; SIM MIND lists the minds you hold.`
4. `SIM MIND Standard PROFILE Sniper, Line` → `Your slot will carry Standard Doctrine (Sniper, Line).`
5. `SIM STATUS` → her row ends `, mind Standard Doctrine (Sniper, Line)`.
6. `SIM START` → no `Humans on the other side` line (asserted absent).
7. Walk to the red room; `BOARD` → `You climb into Red Pod 1. …`, then `Sealed for this sortie:
   Standard Doctrine (Sniper, Line), Class I; MIND reads it.`, then `Match 1 is running. Good hunting.`,
   in that order.
8. `MIND` → at most seven lines, each under 120 columns, naming Sniper and reading "dormant while you
   fly".
9. `MIND PROFILE Hunter` → `MIND refused. Hunter is not sealed on this mind; it carries Sniper and
   Line.`
10. `EXIT`, cold (no `STARTUP`) → `You climb out of Red Pod 1. Red Tank 1 is on its own for the rest of
    the match; you cannot take it back.`; God's `@arena/status` gives the arena clock at the exit
    (`support::arena_clock_seconds`, `:40-44`).
11. Wait for `Match 1 ends: every pilot has left it.`; then `SIM REPORT` → the `Your mind` block: `Your
    mind: Standard Doctrine (Sniper, Line).` and `It flew your tank from m:ss, when you climbed out, to
    the end.`

**The evidence after it, read from the store's tape** (`log:match.1`, the rig's `ledger_events`):
- her hull's `mind_sealed` line carries `card_digest` = SHA-256 of `serde_json::to_vec(&card::preset("Sniper",
  reflex))` over the rig's own arena doctrine — the sealed card is the chosen one, and the fill's lines
  carry Line's;
- her hull's `startup` and `power_online` lines stand AFTER the exit's arena second — the hull was cold
  at the `EXIT`, so only its mind started it (F12, `t1_a_released_cold_tank_starts_itself_up_and_stays_in_withdraw`):
  **the tank flown by it**;
- `no_sixteen_digit_word` over every byte Ada saw (`sim_solo.rs:740-747`);
- the whole run under 150 s asserted (the M1b bound) and under 30 s measured in debug, so the test is
  not `#[ignore]`d (`ci.yml:52-57`; measured on the build container: the M1b transcript 26.84 s,
  `d3_…`, which waits out the same ten-second unattended end, 13.60 s).

`help minds` is not typed: the solo rig serves no help topics (F12); the page's opening is pinned by
`m5_help_minds_opens_with_three_true_lines` instead (M5.6).

### 4.2 (b) Goldens and the bot parser

- **Goldens**, each re-blessed once by its own command and its diff read: `cockpit_frame.json` and
  `view_frame.json` (the `mind` key, `"version": 3`; M5.4); `web_view_model.txt` (M5.5); the four
  `tui_cockpit_*` TUI goldens (the weapons pane's tail; M5.5); `notice_lines.json` (the `profile`
  refusal; M5.2) and `catchup_lines.json` (D14; M5.3), hand-written with both renderers; the canonical
  fixtures test (`tests/cockpit.rs:2933`) green.
- **The bot parser and contract**, `cargo test -p mush-bot` green with: the board's `, a mind`
  (Selection) and `, flown by its mind` (the resume standing); the reconnect line; the `profile` word in
  `every_refusal`; neutral-line cases for the seal line, Q-UM-15's sentence, every `MIND` line and
  sentence, the reworded `SUPPORT` and release acknowledgements and the D14 catch-up; `SIM MIND` and
  `MIND` in `arena_e2e.rs`' `forbidden`; `the_bot_flies_a_match_alone_against_one_machine` (`:135`) and
  the resume flow in `arena_lane` green on the new words.

### 4.3 (c) Fog asserted (C-4, D10, §2.4)

**Ruled.** During selection a slot's mind is read by its own character (and staff), a side's fill by
its lead (and staff); everyone else reads `a mind` against a chosen slot or fill and nothing against an
unchosen one. From `SIM START` on, no mind is named to anyone but the pilot whose hull carries it —
their `SIM MIND` read, the seal line, `MIND` and their own report's `Your mind` block — the account that
set a side's fill (its `SIM AI MIND` read), and staff; everyone else's `SIM AI MIND` reads `The fills
are not shown once a match starts.`, and no board row names or marks a mind for anyone.

**The battery** — `crates/mushd/tests/observer_rule.rs::m5_a_slots_mind_is_read_by_its_own_character_alone`,
on a **`boot_filled` variant** of the file's boot (`tacsim_config` with `cfg.match_.ai_fill = true`, as
`sim_solo.rs:137` sets it; `observer_rule.rs:165-178` leaves `ai_fill` at its default `false`, under
which `SIM AI MIND` is refused `AI fill is off on this server.`, `config.rs:625`), used by this test
alone so no other test's two-tank field moves; Ada the owner and red's lead, Bo the opposing pilot, Cy
the bystander, God as staff (`:1-25`); no test provider (`DoctrineOnly` already refuses any other name):
- **Selection.** Ada sets `SIM MIND Standard PROFILE Sniper, Line` and, as red's lead, `SIM AI MIND
  Standard PROFILE Hunter`. Ada's `SIM STATUS` names her own choice and her fill's; Bo's and Cy's read
  `, a mind` on her row and `red fill: a mind`; Bo's own unchosen row prints nothing to anybody; God's
  `@arena/status` names both (staff, D10); bare `SIM MIND` and `SIM AI MIND` by Bo and Cy print only
  their own (or `Red's fill carries a mind.`); `SIM MIND Sentinel` by Bo is refused as not held;
  nothing of either choice is announced to any room.
- **From `SIM START` on.** No board row names or marks a mind for any reader, staff included; the seal
  line reaches Ada alone; bare `SIM AI MIND` by Ada reads `Red's fill: Standard Doctrine (Hunter).`, by
  Bo and Cy `The fills are not shown once a match starts.`; bare `SIM MIND` by Bo reads his own `Yours:
  none; …`; no `Humans on the other side` line (Doctrine is never substituted); Bo's cockpit frame over
  the web socket (`pushed_cockpit`, `:783`) carries his own `mind` object and no byte of Ada's (M5.4's
  case, re-run).
- **After the end.** `SIM HISTORY` (`history_row_is_public`, `:531`) and `SIM REPLAY 1`
  (`i1_a_replay_is_public_names_nobody…`, `:5167`) read by Bo and Cy name no mind and no preset; Bo's
  `SIM REPORT 1` carries no `Your mind` block (he chose nothing and was never released) and Cy's is
  refused (`o3_…`, `:6514`); Ada's carries hers.
- **The sweep.** Bo's and Cy's own bare `SIM MIND` replies — the only bytes either receives that list
  the presets (`up to two of Line, Sentry, Sniper, Skirmisher, Hunter or Rearguard`) — are windowed out
  by `Term::mark` (`crates/mushd/tests/support/mod.rs:195`) before and after each; **everything else**
  Bo and Cy received, terminal bytes and WebSocket frames alike, is searched for the bare words
  `Sniper` and `Hunter` (so a frame's `"profiles":["Sniper","Line"]`, a staff-style `name="Sniper…"`
  or a future sentence's bare preset fails it, not only the `(Sniper, Line)` and `(Hunter)` forms the
  board prints), and for six-hex and sixteen-hex words (`names_nothing`, `:952`;
  `no_sixteen_digit_word`, `:2843`). Bo's own frame's `mind` object carries `["Line"]` (he chose
  nothing), so it holds neither word.
- **The cheap twin.** `tests/match_lifecycle.rs::m5_the_board_names_a_slots_mind_to_its_own_character_alone_in_selection`
  and `m5_the_bare_reads_follow_the_state` (M5.1), with their post-start halves (Ada as setter, Bo, a
  bystander, staff).

**"In the record", ruled** (C-4): the pilot-facing records — `SIM HISTORY`, `SIM REPLAY`, another
pilot's `SIM REPORT`, the board after the start — name no mind; `MatchRecord.minds`, the tape's
`mind_sealed`/`mind_card` lines and `@arena/report`'s staff fold keep naming it by design.

### 4.4 The sweep and the acceptance

`cargo fmt --check`; `cargo clippy --workspace --all-targets -- -D warnings`; strict rustdoc; `cargo
test --workspace --exclude mush-bot --no-fail-fast`; `cargo test -p mush-bot`; the web lane (`npm ci`,
`npm test`, `npm run build`); the three CI-named `--ignored` minds steps, verbatim as `ci.yml:50`, `:59`,
`:69` run them:
```
cargo test -p tacsim --test minds_rhai m2_the_twin_corpus_hashes_the_same_on_every_host -- --ignored --exact --nocapture
cargo test -p tacsim --test twin m2_whole_match_hashes_are_equal_between_the_runners_over_the_pool -- --ignored --exact --nocapture
cargo test -p tacsim --test minds_pool m4_real_tasks_over_the_twin_s_thirty_hash_as_the_library -- --ignored --exact --nocapture
```
`tacsim_harness_arena_step` once with its lines verbatim and the deterministic block equal; the
fan-out lane's `harness_fan_out:` line with `panel_bytes_max` under 16,384 (M5.4); the five production
pins at their `5f505c8` values; the format at 10 and `af1` at its M5.2 values; the F16 grep's output.
The M4 gate's three weekly runs stay the owner's and are not this gate (OQ-12).

---

## 5. The deploy note

One deploy, at M5.7's gate, and one void. The format move voids every match running across the reload
`aborted(restart)` with `snapshot format 9 is not 10` (AF.3's boot, `arena/snapshot.rs:276-283`;
deploy/README.md); nothing migrates. `LIVE_FORMAT` stays 1 (`match_/record.rs:957`): every live-record
addition — `Slot.mind`, `Centre.fill_minds`, `Centre.doctrine_only`, the report fold's `OwnMind` and
spans, `MindSeal.profiles` — is `#[serde(default)]`, so the void is the snapshot's alone. `MIND_ABI`
and `STANDARD_SHA256` stand, so no seal is refused by name; `FRAME_VERSION` stands at 3 and an older web
client ignores the `mind` key, but the web build ships with the server. **The bot binary is deployed
from the same commit**: an older bot reads `, flown by its mind` as part of a pilot's name and loses its
resume (F10). The drone pack's version 2 → 3 is a content reload. No production hash pin moved, so the
harness baseline and the owner's weekly runs stand. The owner is told in 17b's M5 Deploy cell what the
deploy changes: the choice in the lobby, the seal line, the board's and the reconnect's words, the
`MIND` verb and block, the report's block, the first hint, an unarmed hull no longer refused for a
raised odds floor (C-23), and the help that now says what the machines do (they do not contest a Hold
match's spot; a supporter fights what it finds) until the residues cut restores the behaviour.

---

## 6. The two cuts after M5 (for the record)

### 6.1 The residues cut (after M5; name 17b cut M5r or a 16c row, the committer's choice)

STOP A by a public keyframe key (OQ-7 (a)): the match's mode word and its key spot's cell, letter and
radius on `Frame` and the keyframe (never `held_by` or the clock, `arena/state.rs:852-865`), with
`FRAME_VERSION` 3 → 4 (a field added for a reader that is not a person), a `View` getter and its
`PARITY` row (`minds/api.rs:306`) so `API_VERSION` and `MIND_ABI` move, the native `think` passing the
objective (`arena/stock_mind.rs:766`) and Hold arms written into `standard.rhai` for Gate A, so
`STANDARD_SHA256` moves; STOP F (the motor keeps the goal on `PLACE`, or a mind-only notice, by the
owner's parity reading); STOP G (a wreck's motor row forgotten in `destroy_unit`, `arena/damage.rs:1310-1317`, or
filtered in `fan_goal`, `arena/motor.rs:2505-2524`); STOP I (the doctrine remembers the cell it fired from); STOP K
(arrival judged by the motor's word); M4.5a's (1) and (2) if the owner wants them; the stale
`standard.rhai` header and its "fourteen keys" (`:4`, `:38-39`, `:128`) riding the digest move; the help,
the `SUPPORT` acknowledgement, the drone note and the leader's prompt restored to the restored behaviour,
and the fourteen deleted tests revived from `3a3c469^` where their behaviour returns. It is one
`STANDARD_SHA256` move, one `MIND_ABI` move, one `FRAME_VERSION` move, one declared re-bless of every
production pin with the reason, Gate A re-run, and its own deploy; sequenced against the owner's 16c
T-104 (hash v2, "one declared re-bless of every hash pin … in its own commit", `docs/design/16c-simulator-backlog.md:241`) — before it or
after it, never interleaved. STOP D's designation half and STOP J are accepted divergences under D-UM-15
and §17; Q17's stand is accepted.

### 6.2 The rule-language cut (right after M5's gate; OQ-4)

The pilot's slot grammar on the M1b.4 parser (`match_/cockpit.rs:4925-5165`): the conditional `HOLD
FIRE` veto (spelled as a station form the help contract can hold — `HOLD` is no verb,
`crates/mush-contract-tests/help-commands-allowlist.toml:39-40`); `LOCK nearest|best_odds|weakest|flank|newest WHEN …` rules,
making `SlotTarget::Contact` lawful at the landing for a classified contact with its own sentence
(`arena/conditions.rs:195-215`) and `BestHostile` ranging over classified contacts; the remaining
slot-fitting terms (`target = moving|stopped|hurt`, `target <section> = …`, `rounds <`, `own <section>
<`, `heat >`, `no lock`); more than one slot a weapon with §5's precedence (an `ORDER`'s hold fire, then
a matching `HOLD FIRE`, then the first matching rule, then the sliders); the think-evaluated terms
(`hit within`, `hostiles >=`, `order =`) refused to a pilot in one sentence (a pilot's mind is dormant);
English written back for each form; Q-UM-16's default (a slot is the principal's written shot decision)
confirmed by OQ-4. `FireCondition`'s new fields are its own `SNAPSHOT_FORMAT` move (10 → 11) and `af1`
re-pin, each folded only when not its default so the stock mind's armings in every production match hash
as today; `MIND RULES` and the block's rules line grow with it; `docs/help/cockpit.md:436-446` and the
bot's `m1b4_…` contract (`crates/mush-bot/tests/arena_contract.rs:2324`) move; its own deploy. Rules
SEALED on a profile, `MindSeal.armed`, the card's `R1 … (armed on the core)` / `R2 … (checked each
think)` and the block's rule glyphs wait for M6's authoring, the only place a rule is saved (C-22).

---

## 7. What remains for the owner

**Nothing blocks M5.1.** The owner's rulings of 2026-10-10 settle every question the maps raised. The
two choices the feasibility review left to the lead were **decided by the orchestrator on 2026-10-11**,
each on the review's own recommendation and recorded here as decided, not as the owner's:

- **Where the TUI draws the MIND block — decided: the weapons pane's tail** (M5.5), not a fourth Right
  pane. A fourth pane on the Right column's even `Ratio` split (`tui/app.rs:3364-3424`) costs the
  contacts pane two rows at 120×34 (X4 below the fold) and at 120×30 (X3), undoing 16b cut AB.1's
  budget and moving the content assertions of `the_cockpit_screen_paints_at_both_budget_sizes`
  (`tests/tui_golden.rs:925-1003`); it would also paint an empty pane on the replay and watch screens,
  and `PaneSpec` has no visibility predicate (`tui/screen.rs:205-222`) to give "in the weapons block's
  place on an unarmed hull" a mechanism. The tail costs nothing of that: at 120×34 the weapons pane's
  seven inner rows hold the canonical frame's three weapon rows and the four-row tail whole (read off
  `tests/golden/tui_cockpit_120x34.txt`); at 120×30 the tail's last row (the channel row) is clipped;
  on an unarmed hull the tail replaces `(no weapon)`. The web follows the same placement, so line mode,
  the TUI and the web put the block in one place.
- **Where `Your mind` gets its profile names and whether the slot chose — decided: seeded at check-out**
  (M5.6, `Fold::with_own_mind(OwnMind)` beside `with_own_tank`, from `Side.minds`), not a `profiles`
  key on `mind_sealed`. The tape-shape option moves `EventKind::MindSealed`'s shape (skipped when
  empty, so no pinned line would move, but a shape every tape reader and the staff fold then carry for
  one surface), and it still cannot say whether a `SIM MIND Standard` with no `PROFILE` chose (that
  seal is byte-identical to an unchosen slot's, F4), so the check-out seed is needed anyway; the
  report fold is already seeded with facts the tape lacks (`with_own_tank`, F12), so this is that
  precedent, not a departure.

Five readings of doc 17's prose are taken by this note and declared; the owner may veto any of them
before the sub-cut that lands it, at no cost to the others:
1. **The seal line is said only to a slot that chose a mind** (C-18; M5.2) — §12 says every sealing is
   announced in one line before `STARTUP`; saying it to every seated pilot would add a line to every
   transcript and the bot's lane for the default nobody chose. Veto: say it to every seated pilot.
2. **`EXIT` keeps `, left` on the board** (C-11; M5.3) — §12 reads `flown by its mind` "or on `EXIT`".
   Veto: one word for both, with the bot's `Standing::Left` retired.
3. **The `Your mind` block's "from when" and its shot count are the world's clock** (M5.6) — the one
   report fact not told from the tape or the check-out (16b cut V.3), because the tape carries no
   hand-over line and adding one is a new ledger kind that moves every released-hull fixture's tape; a
   shot in a hand-over's pulse may be counted on either side of it; the pilot's own counts are
   untouched. Veto: the block says only whether it flew, and counts no shots.
4. **Q-UM-15's substitution is taken at the seal, so a pilot's own non-Doctrine mind is never sealed in
   a both-human match** (C-7; M5.2) — doc 17:1489-1490 keeps "a pilot's own sealed mind … dormant on
   release" and sends the hull to Doctrine; this note seals Doctrine instead (with `MindSeal.fallback`
   naming why), because a hull carries one seal (`SealRef`, `MatchRecord.minds`) and the arena flies its
   seal — a second, dormant sealed mind per hull is a shape no record carries. The consequence for the
   record: `MatchRecord.minds`, the tape's `mind_sealed` line, the seal line and `MIND` name Standard
   Doctrine for that hull, never the pilot's own choice. Fixture-only in M5 (OQ-5: no production match
   substitutes), but M6 inherits the shape. Veto: carry the pilot's mind sealed and a Doctrine fallback
   beside it, which is M6's shape.
5. **The card's slider drawing (R-25) and the rule glyphs (R-24) move to M6, under OQ-8** (C-22; M5.4,
   M5.5) — doc 17:983-988 says "the card draws the same way everywhere, one slider a line with a
   10-cell bar and the unit-aware value" and "the first three rules carry a state glyph"; this note
   reads OQ-8 ("the block shows the seal facts, profile, channels and rules") as the whole of M5's
   block, so M5 draws no card on any surface ("the same way everywhere" holds vacuously) and the glyphs,
   which have no sealed rule to mark before M6's authoring, land with the first sealed rule. Veto: M5.4
   and M5.5 draw the active profile's card (one slider a line, a 10-cell bar, the unit-aware value) in
   a `MIND CARD` read on line mode, the TUI and the web — a new form and a new keyframe key for the
   card; the glyphs still wait for a sealed rule.

One behaviour change is doc 17's text, not a departure, and is recorded: an unarmed hull is never
refused for the odds floor (C-23).

---

## 8. For the committer

### 8.1 Where the record goes (17b §0.7)

- **17b**: the M5 cut-map row (17b:60) revised to §8.2 expanded (the committed row is §8.2 with its details spelt out) in this note's commit; the ledger's M5 row
  appended with the design note (this note, condensed into one cell in the M4 row's order: the head, the
  findings, the settled questions with the seven sub-cuts named in bold, "For the committer"), filled
  at the record with Landed, Evidence (the loop, the gates of §4 verbatim, the plan's acceptance claim by
  claim, the F16 grep's output, the fan-out lane's `harness_fan_out:` line) and Deviations (the owner's
  rulings of 2026-10-10 quoted — **OQ-6's "D's designation half recorded as accepted under D-UM-15"
  quoted in its own sentence**, since it is an acceptance the record must carry, not a scope line —
  the five readings of §7 and whether any was vetoed, the two choices the orchestrator decided (§7),
  the splits if any, Deploy, residuals still open: D16, D17, the residues cut, the rule-language cut,
  the name of a non-Doctrine seal on the keyframe, **the card's slider drawing (R-25) and the rule
  glyphs (R-24), both M6's** (C-22)); §2 gains a row only if the harness block moves (it must not).
- **Doc 17 §22**: the M5 paragraph gains one sentence pointing at 17b's plan in this note's commit, and
  at the record the milestone line "Landed <date> at <hash>; see 17b section 3, row M5."
- **Doc 17 §24**: one bullet per sub-cut, as built; and the corrections and rulings this note takes —
  C-1 (D-UM-10's count read as 17), C-4 (the fog's "anyone else": the pilot's own surfaces, the fill's
  setter and staff are not), C-5 (§15 :1129 and §23 :1741's "no mind" as "no mind of the pilot's own"),
  C-6 (a profile row's click prefills `MIND PROFILE`), C-7 (Doctrine at any preset; the fill at the
  lead's chosen preset; the substitution at the seal), C-8 (`SIM MIND` prints both the refusal's and the
  bare read's arithmetic), C-11 and C-11a (`EXIT` keeps `, left`; sortie and match), C-12 (§15's first
  hint as reworded; the second hint moved to M6), C-13, C-14, C-15 (§9's example, OQ-11's sentence),
  C-16 (§9 :755), C-18 (the seal line to a choosing slot), C-19, C-20 (`IntentKind::Profile`, seat-only,
  continuous under its own key), C-21 ("M5's authored minds" at :1800 and :1805), C-22 (the card drawing
  and the rule glyphs are M6's, under OQ-8), C-23 (R-60 at the seal, on Q-UM-12's default), C-24
  (over-class exactly), and the "one list"'s omissions (C-10); and the owner-ruled question rows'
  outcomes: Q-UM-13 (holdings keyed by the character who boards, confirmed by OQ-2), Q-UM-15 (ruled by
  OQ-5, its four sub-questions answered as §2.3 says), Q-UM-16 (confirmed by OQ-4, landing in the
  rule-language cut), Q-UM-3/D-UM-37 (the lead's fill class deferred to the cut that rates a mind above
  Class I, OQ-10 (b)), Q-UM-2 (C-8).
- **Doc 17 §21** (the questions' own list, `17-unit-minds.md:1469-1503`): the Q-UM-13, Q-UM-15 and
  Q-UM-16 entries each gain "Ruled by the owner 2026-10-10 (OQ-2 / OQ-5 / OQ-4): the default holds" (for
  Q-UM-15, "… 'Doctrine' is Standard Doctrine at any preset; the substitution is built for non-Doctrine
  seals and taken at the seal"); Q-UM-12 is marked "the default relied on by 17b cut M5 (C-23),
  unruled".
- **Doc 16**: D-16-3's row (doc 16:1156) gains D-UM-50's clause verbatim (doc 17:1360-1362: "an OOC match
  posts nothing to, consumes nothing of and reads nothing from the IC world but D-UM-49's mind
  holdings"); §3.7's verb table (doc 16:559-577) a `MIND` row, and its lobby list (:590-609) `SIM MIND` and
  `SIM AI MIND`; `flown by doctrine` at doc 16:667 and the Y.3 reconnect sentence at :1078; §11 entries
  for the board's words, the pool's wording, seated hulls at the pool, STOP E's charge, the format move,
  the unarmed hull and the floor (C-23), and doc 16:1139's "caps nothing until M5 rates minds" (C-21).
- **16c**: no row closes; T-104's sequencing note names the residues cut.

### 8.2 17b's M5 cut-map row, revised (the cells after "M5")

**Scope**: The pilot's experience (planned 2026-10-10 from `5f505c8`, revised the same day after
review and reconciled 2026-10-11 against both reviews read whole, seven sub-cuts; the lead's note binding as the owner adopted it 2026-10-10: no licence, market,
library or workshop; no Rhai on a production path; no `STANDARD_SHA256`, `MIND_ABI` or `FRAME_VERSION`
move and no production hash pin moved; one `SNAPSHOT_FORMAT` move; one deploy at the gate; the rule
language its own cut after the gate and the doctrine and motor residues a cut after M5): a holdings
seam holding Standard Doctrine and its six presets for every character (a test provider holds two
scripted fixture minds, a rated-I `Fixture I` and the rated-III `Load III`, and installs their sources);
`SIM MIND` and `SIM AI MIND` with profiles by the sealed class, fitted at the choice (class, over-class
on a governed core, the floor on an armed hull, count, the pool counting seated hulls; the lobby's own
refusals reused, a holding lost before the check-out calling the match off by slot) and fogged per
state to the slot's own character, the fill's setter and staff, a change of lead clearing the side's
fill choice; the choice sealed at check-out with every default seal byte-identical, `SealRef.profiles`,
`UnitState.card_profile` and `IntentKind::Profile` (seat-only, continuous under its own key, its own arm
before the power and motion gates), STOP E's charge and the `af1` inner shapes in `SNAPSHOT_FORMAT` 10;
the released hull flying its sealed profile, the `Withdraw` seed gated on the doctrine bit, Q-UM-15's
substitution for non-Doctrine seals at the seal; the seal line to the choosing pilot alone; `flown by its
mind`, the reconnect, the release acknowledgement and D14; the seventeenth cockpit verb `MIND` and `MIND
PROFILE` with `Frame.mind` (the core class read live) compared in `same_as`, the block on line mode
and as the weapons pane's tail on the TUI and the web at 120 columns (the card's slider drawing and the
rule glyphs M6's, under OQ-8); `SIM REPORT`'s `Your mind` block, its span line bounded; the first hint;
`help minds`; the help, the `SUPPORT` acknowledgement, the drone note and the leader's prompt made true.
**Depends on**: M4 (the flip, the seal on every hull, the doctrine bit, `SNAPSHOT_FORMAT` 9, the native
fill); M3 (the seal, the pool, the class table, the core); M1b (the card, the presets, the slots,
`@mind/set`); M1a (the cockpit verb set, `DRIVE`'s precedent); 16b cuts Q.8 (`SIM UNIT`'s model), Y.3/Y.4
(the drop, the reconnect, the catch-up), V.3 (the report), AF (the snapshot), AB.1 (the TUI budget),
AH.1 (the hints). **Size**: Large, seven sub-cuts. **Acceptance**: the line-mode transcript on the local
rig of a pilot sealing Standard Doctrine on Sniper and Line, reading `MIND`, climbing out cold and the
tank started and flown by its sealed mind (its `mind_sealed` card digest Sniper's, its `startup` after
the exit, the report's `Your mind` block), under 30 s; the goldens re-blessed with `FRAME_VERSION` 3 and
the bot's reader and contract green on every new sentence, `SIM MIND` and `MIND` forbidden to the bot;
the fog battery on a filled boot (selection reads by own character or lead, `a mind` or nothing for
others, staff reading; after the start only the pilot, the fill's setter and staff; history, replay and
other reports clean; every byte of Bo's and Cy's, frames included, swept for the bare words `Sniper` and `Hunter` (Ada's chosen presets)); a default seal digesting as at `5f505c8` and the
five production pins unmoved; the format at 10 with `af1` re-blessed once and the Support fixtures moved
once; a profile switch one event list and one hash sequence at 5,000, 1,000 and 500 ms and across a
resume, and one frame on a quiet hull; the fan-out lane's keyframe bytes under its bound; every
remaining "17b cut M5" comment true; D-16-3's clause in doc 16; the full sweep with the web lane.

---

## 9. The reviews' findings and their dispositions

Both reviews read the draft against `5f505c8`; nothing in the tree was edited. Every finding was
checked in the code before it was taken. Review B's text reached the first revision cut off in its
sixth finding, and its later findings were applied from its summary alone; **on 2026-10-11 both
reviews were re-read whole** (`briefs/M5-adversarial-full.md`, 25 findings; `briefs/M5-feasibility-full.md`,
22 findings) and every finding checked against this note, the conventions brief and the 17b rows
(`briefs/M5-review-reconciliation.md`). Review A's 22 had all been taken. Of Review B's 25, ten had
been taken whole; eight were not applied, all among the ten notes the first revision never saw (B16,
B18 to B22, B24, B25; of the other two, B17 was already answered by A8's fix and B23 in part); and seven
were taken only in part (B7's
Q-UM-12 citation, B8's OQ-8 citation and residual, B10's width test, B11's missed sites and wider grep,
B13's §21 marking and OQ-6 quote, B15's already-active prefix and declared split, B23's test). All
fifteen were completed on 2026-10-11. B-numbers below follow the full text's order (B*n* is its finding
[*n*−1]); the first revision's B7 to B15 were the summary's order.

**Review A (feasibility): no blocking defect, 13 should-fix (its summary counts twelve; the finding list holds thirteen), 9 notes — all taken.**

| # | finding | severity | disposition |
|---|---|---|---|
| A1 | `UnitState.profile` collides with `profile: UnitProfile` | should-fix | renamed `UnitState.card_profile: ActiveProfile` throughout (F5, D2, M5.2 items 3, 5, 8, 9, M5.4 item 2, §5, §8.2); `every_unit_field` 43 → 44 listed; the never-folded probe gains `seal.profiles` |
| A2 | the over-class refusal breaks two tests and, read literally, every coreless hull | should-fix | C-24's exact rule in M5.1 and M5.2; the two tests rewritten and listed; `m5_a_range_check_out_seals_its_coreless_target` added |
| A3 | `IntentKind::Profile` needs its own coalescing key and must miss the power and motion gates | should-fix | `continuous_code` 5 (discrete stays 4); its own arm before the continuous gates; a cold or immobile hull switches; tests for a cold tank and a same-pulse heading |
| A4 | `Frame::same_as` decides whether `mind` is ever re-sent | should-fix | `same_as` compares `FrameMind: PartialEq`; `switch_at_ms` absolute; line mode's countdown against `MatchMirror.arena_ms`; the stationary-hull test |
| A5 | a fourth Right pane costs contact rows at both floor sizes; no conditional pane exists | should-fix | the preferred fix, **decided by the orchestrator (§7)**: the block is the weapons pane's tail on the TUI and the web; no pane, no split, no contacts row moves; replay and watch draw no tail |
| A6 | integration rigs cannot store the fixture mind's source | should-fix | `HeldMind.source`; `install_holdings` installs it through `install_mind_source` split out of `put_mind_source`; `load_iii.rhai` reused, not copied |
| A7 | the tape carries neither the profile names nor whether the slot chose | should-fix | the second option, **decided by the orchestrator (§7)**, on `with_own_tank`'s precedent: `Fold::with_own_mind(OwnMind)` seeded at check-out from `Side.minds`; no tape shape changes |
| A8 | `SIM AI MIND` is refused on observer_rule's boot | should-fix | the `boot_filled` variant for the battery alone; the test-provider install dropped from the battery |
| A9 | web files change in sub-cuts whose gates run no web lane | should-fix | §3's preamble names the web lane for M5.2, M5.3, M5.4, M5.5; each gate evidence names it |
| A10 | the drone digest's second pin | should-fix | `tests/extensions.rs:10788` listed in M5.6 with "Was `1bfeca3a…` at version 2"; the [U] dropped |
| A11 | the over-subscribed roster test asserts the old `StartError::Pool` | should-fix | `tests/match_lifecycle.rs:9678` listed in M5.1 |
| A12 | duplicated rating and engine on `SealTerms` | should-fix | `SealTerms.identity: SealIdentity` (five identity fields only); `rated`/`engine_for_minds` authoritative, copied from the held mind at `start` |
| A13 | the staged refusal sentence is composed from `as_str` | should-fix | `as_str` stays `mind`; the sentence is `MIND refused. That profile cannot be switched to now.`, and the local pre-checks open `MIND refused.` too; no renderer special-cased |
| A14 | the prefix shift resolves with no kernel word taken | note | recorded in M5.4 item 7; `m`, `mi`, `min` → `Ok("mind")` pinned; the [U] and the STOP clause dropped |
| A15 | `view_frame.json` embeds the status object | note | unconditional in M5.4 and §4.2 |
| A16 | the truth-serialization test cannot see the key | note | the [U] dropped (M5.5) |
| A17 | a chassis's core class is readable without an instance | note | `core_of_type` (class and governed) beside `class_of_type`, and `armed_of_type`; the [U] dropped |
| A18 | why `Refusal::Profile` needs no `API_VERSION` bump | note | stated in F6 and M5.2 item 4 |
| A19 | Load III seals only on the heavy | note | taken with B1: a rated-I `Fixture I`; Load III kept for the over-class refusal only |
| A20 | the seal line's recipient | note | to the pilot alone through an `Audience` (M5.2 item 12, D7) |
| A21 | `MindCard`'s documented contract and its world reader | note | the doc amended in M5.2 item 4; M5.6 prints every switch by its card's name and says staff edits appear |
| A22 | claim (4) can be closed by reading | note | the [U] replaced by the citations in M5.2's "Why no production hash pin moves" |

**Review B (adversarial completeness), read whole: nothing blocks M5.1; thirteen should-fix and twelve notes — all taken.**

| # | finding | severity | disposition |
|---|---|---|---|
| B1 | the one fixture mind cannot reach the paths it is named for | should-fix | `Fixture I` (rated I, a few ops) added for every non-Doctrine path; Load III for the over-class refusal only; each lobby fixture names its chassis (`line_tank`) and `fill_class` (I) (M5.1 item 1) |
| B2 | the fog battery's boot has the AI fill off | should-fix | as A8: the `boot_filled` variant for the battery alone, its fill hulls confined to it (§4.3) |
| B3 | no state rule for bare `SIM AI MIND`; the post-start fog leaves out the lead | should-fix | §2.4's table; §4.3's ruled sentence names the fill's setter; post-start bare reads by Ada, Bo and Cy in the battery and in `m5_the_bare_reads_follow_the_state` |
| B4 | the fill's choice across a change of lead | should-fix | `FillChoice.by`; `resettle_leads` clears a side's lead-set entry when the lead changes before the check-out; `m5_a_change_of_lead_clears_the_sides_fill_choice` (§2.4, M5.1 item 5) |
| B5 | `Frame::same_as` not told about `Frame.mind` | should-fix | as A4 (M5.4 item 2; `m5_a_switch_on_a_stationary_hull_sends_one_frame_and_mind_reads_it`) |
| B6 | taking the mind's shots out of the pilot's rounds and kills is undeclared and timing-dependent | should-fix | the preferred fix: the pilot's `rounds`, `kills`, `You fired` and timeline untouched; the block alone counts the span's shots, the pulse tolerance declared with §7 3; a test holds the pilot's counts at their `5f505c8` values (M5.6 item 1) |
| B7 | R-60: the fire section on a hull with no weapons must never refuse the seal; Q-UM-12 uncited | should-fix | C-23: the floor probe only on an armed hull, at the choice and the seal, with its two tests; **Q-UM-12's default (D-UM-48) cited** in C-23 and §1.2's R-61 row as the reading M5 relies on, and marked in doc 17 §21 at the record (§8.1) |
| B8 | R-25's card drawing and R-24's rule glyphs dropped under the wrong ruling | should-fix | C-22: placed in M6 **under OQ-8** (the OQ-3 citation withdrawn); §1.2's row; **§7 5** on the veto list; carried as a residual in 17b's Deviations (§8.1) |
| B9 | Q-UM-15: the fill flies Doctrine at the lead's chosen preset; the substitution at the seal undeclared | should-fix | C-7 and M5.2 item 11 (the fill at the lead's chosen preset when it is one of Doctrine's six); **§7 4** declares the seal-time substitution and its consequence for the record |
| B10 | the "Fits" line runs 123 columns on `line_tank_thermal`, 121 on `line_tank_laser` | should-fix | reworded `Fits <chassis> (<class>): <mind>, up to two of … or Rearguard.`, 115 at the longest stock type word (re-measured over all eleven); **the width pinned by `m5_every_sim_mind_line_is_under_120_columns_on_every_fieldable_type`** (M5.1 item 3, gate evidence) |
| B11 | ~20 "before/until 17b cut M5" comments go stale | should-fix | F16: **58** hits by the widened grep (`M5's`, `M5 admits`, `web/src`, `docs/help` added), each assigned to the sub-cut editing its file — `match_/mod.rs:1179` (M5.1), `arena/conditions.rs:146` (M5.4), and `arena/motor.rs:491`, `arena/stock_mind.rs:93`, `tests/reflex.rs:610`, `:10564`, `tests/shipped_scale.rs:21` (M5.7) added; the grep's output recorded in the Evidence cell |
| B12 | the web tests missing from the gates of sub-cuts that edit `web/` | should-fix | as A9 (§3's preamble; M5.2, M5.3, M5.4, M5.5) |
| B13 | doc 17's record list misses C-4, C-6, C-8, C-11, C-18, C-20, the §21 Q-UM rows and OQ-6's acceptance | should-fix | §8.1 lists every C-ruling; **doc 17 §21's Q-UM-13, Q-UM-15, Q-UM-16 entries marked "ruled by the owner 2026-10-10 (OQ-n)"**; **17b Deviations quotes OQ-6's acceptance of D's designation half under D-UM-15 in its own sentence** |
| B14 | `MindSeal.armed`'s new doc names the wrong later cut | note | M5.2 item 1: "none before 17b cut M6, whose authoring seals a profile's rules" |
| B15 | how `MIND PROFILE refused.` is composed; B-16's one-string rule | note | as A13: composed through `order_refused` with `as_str` `mind`, every refused switch (cockpit or arena) opening `MIND refused.` — the already-active case included; the specific-versus-generic split declared as `NoJammer`'s precedent (M5.4 item 5) |
| B16 | the fog sweep's search strings are too narrow | note | §4.3: Bo's and Cy's own bare `SIM MIND` replies windowed out by `Term::mark`; everything else, terminal and WebSocket frames, swept for the bare words `Sniper` and `Hunter`, plus six- and sixteen-hex |
| B17 | installing the test provider in the fog battery is unexplained | note | as A8: dropped (`DoctrineOnly` refuses any other name) (§4.3) |
| B18 | lobby refusals and re-checks left to guesswork | note | §2.4 and M5.1 item 2: `SIM AI MIND` in `SIM AI`'s order (`AiOff`, `AiOnRange` and the range's own bare read, `WrongState`, `NotLeadForAi`); `SIM MIND`'s set `WrongState`/`NoSlotToField`; M5.1 item 6: `SimError::MindNotHeld`, called off by slot; M5.1 item 4: only `SIM UNIT` and `SIM MIND` re-run the pool, a claim, `SIM AI <n>` and `SIM AI MIND` left to the check-out's gate and unable to refuse at the stock constants; tests named |
| B19 | `MIND` prints the sealed core class, not the live one | note | M5.4 item 2: `FrameMind.class` = `Arena::mind_class`, `core_class` = `Arena::core_of`'s, both live; the seal line keeps the sealed figures; `same_as`'s reason updated; `m5_the_frames_mind_reads_the_core_class_now` |
| B20 | the `Your mind` block's span line has no bound | note | M5.6 item 1: one and two spans as before; three or more `It flew your tank N times: first …, last ….` (≤ 86 whatever N); `m5_the_span_line_is_bounded_at_five_spans` |
| B21 | "the three named `--ignored` minds steps" are not named | note | §4.4 lists the three `ci.yml` commands verbatim; the conventions brief §5 too |
| B22 | two of `help minds`' three pinned opening lines are inaccurate | note | M5.6 item 3: line 1 says `SIM MIND` chooses and the start seals; line 3 says a drop hands over past the grace and only a drop is boarded back (112, 117, 108 columns) |
| B23 | the world's mirror card after a profile switch is unspecified | note | M5.2 item 4 already said `note_cards_from_tape` absorbs a seat's `mind_card`; the test added: `m5_a_staff_read_back_follows_a_pilots_switch` (M5.4 gate evidence) |
| B24 | three design-map requirements have no row (R-9, R-27, R-51) | note | §1.2 rows: R-9 → M5.1 item 4; R-27 → M7, vacuous; R-51 → the histogram already prints a state word, fixture-only, noted with D14 |
| B25 | the fan-out harness's keyframe-bytes bound is not re-read | note | M5.4's gate quotes `harness_fan_out:`'s `panel_bytes_max` and `events_per_pulse_max` on M5.3's tree and M5.4's against 16,384; §4.4 and the 17b acceptance carry it |

**Found by the first revision** (re-measuring B10): `StartError::Pool`'s long form `{callsign}: This tank
would think …` runs past 120 columns at six-figure terms with a long callsign. It is the log's and the
fixtures' long form, which no pilot reads (`call_off_words`' doc, `match_/mod.rs:891-897`); M5.1 item 8
says so, and the short call-off form stands. The same re-reading found Q-UM-15's `SIM START` sentence
written actor-only (the actor is whoever typed `SIM START`); it is told to each affected pilot alone
(M5.2 item 11), as A20 rules for the seal line.

---

## 10. Unverified, carried (each settled by the sub-cut named)

- That the five production pins stand through M5.2 and M5.4 — the design's claim (F4, F5, M5.2's eight
  reasons), proved by running them before any re-bless (M5.2, M5.4, M5.7).
- That no harness pins the leader's prompt bytes (M5.6's run).
- The exact report pins that gain the `Your mind` block (M5.6's run names each).
- G10's "no fallback-preset configuration" is grep-level (M5.2: a configuration found would be read
  instead of the constant Line).
- The deploy state of M4.3 or later (STOP A's live effect; the residues cut's).
- Whether any pinned production fixture wrecks a hull holding a goal, or places a mind (carries Q6;
  the residues cut only).
