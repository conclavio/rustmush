# 10a — TacSim E7: owner questions

**Draft 0.1 — 2026-09-03.** The survey the owner answers before the E7 implementation plan is
written, in the shape of [08e](08e-e6-owner-questions.md): numbered questions, lettered options with
a **recommended** default, and the code fact each recommendation rests on. Answer `1A, 2B, …`, or
`defaults` to take every recommendation. Grounded in the tree at `34a53c0` (main, K3 merged); every
`file:line` was read.

---

## 1. Purpose and what is already decided

Doc [10](10-tacsim-foundations.md) is binding and this survey does **not** re-ask any of it: the
foundational decisions **1A–7A** (§12 — abstracted fusion routes; continuous pulse combat; risk-bound
loss with a recovery floor; ten commodities and a short chain; risk zones plus declared operations;
journeys never pause for a scene; funded NPC corporations), the character-first decisions **C1A–C30A**
(§18), the addenda **T1–T16** (§13, §19), the sharpenings **R1–R7** (§21), the crate boundary (§14),
the E4–E6 implications (§15) and the environmental seam (§20) are all settled and are cited here as
givens. What remains open is doc 10 §16: **items 1–8 are definition decisions** with no answer yet,
and **items 9–15 are accepted requirements** that need decomposing into build cuts. This document
asks eleven questions — one per §16 definition item, plus three that §16's accepted requirements and
§21 force but doc 10 never decides — and proposes a cut structure (§4) and the handback to the
economy programme (§5). Where an option contradicts a locked decision it is named as such and
rejected, not re-litigated.

## 2. Grounding: what the tree has today

- **Journeys are plain-object routes**, `Journey` keyed by traveler (`mush-sim/src/route.rs:67-90`), and a traveler must
  be plain and **un-placed** — a domain, interior, mount or station host is refused (`mush-sim/src/commands.rs:1529-1563`).
- **Station assignments are host-keyed** (`HostAssignments` over `StationAssignment`, `mush-sim/src/data.rs:446-462`);
  the derived index is actor → *set* of units (`:502-503`) with no active focus, and `StationSpec` grants an explicit
  channel set (`:414-435`).
- **Links are a pure projection of assignments**, unioning channels and taking **max** priority across an actor's
  seats on one unit (`mush-sim/src/links.rs:40-55`) — the hazard doc 09 C8 answers with `ChannelGrant` (`09:989-991`).
  **Control epochs** are per-unit and monotonic (`data.rs:529-533`, `608-640`); `DomainClock::Passive` is the only
  mode, so nothing ticks tactically (`:210-241`).
- **`TacticalUnit` is compositional** — a private-field, serde-re-validated composition of sections, facets,
  `ScalarCell`s addressed by `CellAddr`, components, capabilities and mobility (`mush-tactics/src/unit.rs:361-373`),
  with `InstalledComponent.asset: Option<ObjId>` the economic-asset pointer (`:328-352`).
- **Fronts and control kinds exist**: a `Front` and a `Clock` that can flip `(RegionId, ControlKind)` on completion
  (`mush-faction/src/sim.rs:53-98`); `ControlKind::{DeJure, DeFacto, Covert, Economic}` on a per-region `ControlRecord`
  (`data.rs:420-486`); `backing_org`, re-validated at every use (`econ.rs:137`).
- **Contracts carry outcome tables, roles and audience**: `ContractTerms.outcomes: Vec<OutcomeTerms { name, when,
  payouts, expense_treatment, asset_consequences }>` (`mush-economy/src/contract.rs:235-276`); separate
  `author`/`payer`/`representative`/`facilitator` (`:530-536`); a `ConsentSurface` naming visibility, scope,
  adjudicator and evidence (`:450-480`); `EconomicVisibility::Audience`, `Encumbrance::Lease` (`record.rs:348-364`, `530-537`).
- **Obligations run on cadence clocks, manual included** (`obligation.rs:228-271`; `CadenceClock` with a
  `source`/`period`/`catch_up_cap`/`position` and `advance_cadence_manual`, `clock.rs:208-249`, `300`); `next_due` is a
  **period index**, "never on an IC:OOC ratio or a literal calendar" (`clock.rs:219-225`).
- **The fact seam is already shaped for sim/tactics**: `ExternalRef { provider, kind, id, revision }`
  (`record.rs:307-316`) resolved by `FactProvider::resolve(w, kind, id, revision) -> Option<FactState>` over a
  wiring-frozen table where unknown = unsatisfied (`facts.rs:13-33`, `62-109`), only `mush-jobs` registering so far
  (`modules.rs:148`); canon **outcome** kinds are producer-registered composition-root data, none yet (`contract.rs:296-322`).
- **The C8-shaped reservation in code is a guard, not a store**: `AuthorityGuard::Reservation { provider: OwnerId, id:
  IdempotencyKeyText, revision: u64 }` — no `Deserialize`, coordinator-created, routed to a per-`(owner, kind)` validator
  registered at wiring (`mush-core/src/action.rs:1036-1064`, `1420-1445`; `module.rs:1792-1795`). No `ReservationRecord`
  or coordinator exists: doc 09's record (`09:997-1009`) is design vocabulary.
- **Scene time is start + rounds, with no floor**: `scene_ic_time() = ic_started_at + (round − 1) × round_seconds`
  (`mush-scenes/src/data.rs:171-178`); each `SceneEvent` stamps `ic_time` at emission (`:252-269`).
- **Fungibles are already an economy primitive** (`InstrumentKind::{Currency, Commodity, Requisition, Entitlement}` with
  a `scale`, 0 = indivisible, `record.rs:28-61`); **contact vocabulary exists with no producer** (`ContactId`,
  `PerceivedRef::Contact`, `mush-core/src/action.rs:142-147`, `656-669`); **capabilities are open canonical strings**
  behind a wiring-frozen typo catcher, not a gate (`mush-org/src/ids.rs:122-126`).
- **The account layer is store-side and name-keyed** — `AccountRec` at `account:{lowername}` holds `characters:
  Vec<CharacterRef>` (`mush-store/src/types.rs:718-741`), yet a character *is* the login (`mush-core/src/obj.rs:90-91`).
- **Absent entirely** (repo-wide grep of `crates/*/src`): `Formation`, `TaskForce`, `TacticalOrder`, deposit, recipe,
  lot, `mush-industry`, `mush-market`, `mush-propagation`, `ruleset-tacsim`.

## 3. The questions

### 1. Extraction and yield model (§16 item 1)

**1A — deterministic yield curve per deposit, with depletion (recommended).** A `Deposit { id, region,
commodity, grade, accessibility, reserve, curve, surveyed_bands }`; a survey action reveals *bands*
(grade class, reserve class), never the exact reserve, and a revealed band is a recorded fact. Claim
tenure and licences are `InstrumentKind::Entitlement` accounts issued by a government principal with
expiry on a campaign cadence (`mush-economy/src/record.rs:52-61`). Production is a **scheduled,
journaled lot creation** on a deterministic key — doc 09 C8 already lists "production, upkeep, and
scheduled obligations" among the bounded-idempotency-key actions (`09:1019-1028`), so no new
scheduling concept is needed.

**1B — stochastic yield seeded per period.** Worse: T1 forbids wall-clock randomness changing a
contract after publication, so a supply contract written against a mine whose output rolls each period
cannot freeze its route assumptions, expected arrival and late terms; and the RNG would land in the
*economic* journal, which T2 places in the mechanical journal instead. Doc 10 4A already supplies variation
the right way — "deposits vary by yield, quality, accessibility, location" — which is heterogeneity,
not per-period noise.

**1C — abstract infinite deposits with licence fees.** Rejected by doc 10 4A and §5 (extraction conflicts and
comparative advantage need scarcity), and by doc 10 7A's rule generalized: an infinite deposit is an
uncontrolled commodity faucet.

### 2. Lot representation (§16 item 2)

**2A — instruments for fungibles, lots with immutable lineage for the rest, ruleset decides per
commodity (recommended).** Propellant, volatiles and bulk metals register as
`InstrumentKind::Commodity` at `scale: 0` (`record.rs:38-39`, `52-61`), so holds, escrow, obligations,
available-balance enforcement, contracts and the disclosure filter all work with **zero new
machinery**. `mush-industry` owns `Lot { id, commodity, quantity, grade, legal, lineage, custody }`
for graded ore, salvage, licensed munitions and anything carrying a legal-status flag. Coarsening
summarizes only the *active* projection; the lineage record is append-only (T5). Interconversion is
one journaled, attributed act each way — `dissolve(lot) → credit a commodity account` (provenance
deliberately discarded) and `crystallize(account, n) → lot whose single parent is the fungible pool`.
This is T5 restated ("the ruleset or recipe decides which representation applies") and matches 08e
answer 9, which already made representation a **catalog policy** rather than a price line.

**2B — everything is a lot.** Worse: every propellant top-up becomes a lineage write and a depot's
inventory becomes thousands of records; and none of E4's money machinery applies, so `mush-industry`
re-implements holds and escrow — the "second semantics" T4 exists to prevent.

**2C — everything fungible with a provenance tag.** Worse: a tag is not lineage. T5's "the immutable
lineage is never discarded" is unsatisfiable, and stolen/embargoed status does not survive a merge.

### 3. C8 interim (§16 item 3)

**3A — `mush-industry` ships a reservation record shape-conformant to C8, with a named migration
trigger (recommended).** This is 08c §12's conformance strategy applied a second time. The record
carries doc 09 C8's exact fields — `{ id, owner, resource, holder, amount, revision, expires, state:
Held | Consumed(commit) | Released | Expired }` (`09:997-1009`) — lives in `mush-industry`'s own
documents, and registers an `AuthorityGuardKind::Reservation` validator under its own `OwnerId`
(`mush-core/src/action.rs:1060-1064`; `mush-core/src/module.rs:1792-1795`) so a K3 action can already
name one; the registration pattern is tactics' `brace` provider — state family, exact definition,
`ActionProvider` (`mush-tactics/src/module.rs:76`, `539-556`, `600-635`). **"Consume atomically" before K3's mixed write sets** means: one plan, one batch, one
journal-authoritative work-order document that names every reservation it needs, with the terminal
step a single companion-carrying batch and no fallible step after the first install — E4b2's
plan-first terminal-transition rule, which is the strongest atomicity the tree offers today.
**Migration trigger**, stated in the plan and not left to judgment: 08c §12's own K3 list —
**mixed write sets**, a **`Ctx` family read path**, and **the seal folded into the bundle**. When all
three land, the states rename onto `ReservationState` and the record moves to a K2 state family, the
same mapping 08c already wrote down for `Hold`.

**3B — wait for the kernel coordinator.** Worse: 08c §12's K3 review names three blockers with no
owner and no date, and refused the same migration for `obligation:{id}` / `contract:{id}` on those
grounds. E7a would be gated on work outside the economy programme, while §11's proving story needs a
refinery slot and a drydock reservation immediately.

**3C — reuse the economy `Hold`.** Forbidden by T4 and doc 10 §15: "Holds stay money-only … must not
grow a resource field."

### 4. Propagation layer (§16 items 4 + 14, R4)

**4A — one `mush-propagation` crate owning contacts *and* delivery envelopes over a shared
reachability step (recommended).** Minimum deterministic model, no belief engine: per-domain
`ContactRecord { observer, subject, first_seen, last_seen, confidence_band, revealed_attributes,
generation }` keyed by the kernel's existing `ContactId` and projected as `PerceivedRef::Contact
{ provider, id, generation }` (`mush-core/src/action.rs:142-147`, `656-669`) — the projection
vocabulary is already in core and only a producer is missing. `Envelope { id, issuer, recipient,
payload_ref, revision, state: Queued | Transmitted | Delivered | Acknowledged | Expired | Superseded,
path }` carries T15's order lifecycle and C9A's consequential communications on the *same* answer to
"has this reached you?". Propagation runs on the domain tick; the belief engine stays out of scope
(§14 splits the dependency exactly this way — (a) is required before space combat, (c) follows later).

**4B — contacts in `mush-sim`, delivery in a separate order subsystem.** Worse: R4 was accepted
because two substrates answer the same question differently — a jammed unit that can still receive
orders, a courier that can be intercepted but never detected — and it duplicates the reachability
computation on the hot path.

**4C — both inside `mush-sim`.** Defensible and cheaper: domains and the tick already live there
(`mush-sim/src/data.rs:210-241`). Against it: `mush-sim` is already the largest sim crate, and
propagation has consumers with no spatial dependency (market intelligence, doc 10 §18.2). Take this if you
prefer fewer crates over a clean consumer boundary.

### 5. Campaign/warfare layer ownership (§16 item 5, R1/R2)

**5A — a new neutral `mush-campaign` crate over faction + economy + org (recommended).** It owns
**one** record that is both the declared operation (T6) and the task force (T8), per R1: roster and
command overlay on one side, objective/window/bond/stakes on the other. Freeze-at-activation reuses
E4b2's consent-surface hash; the bond is an economy hold placed by the attacker's own authority; the
audience class is `EconomicVisibility::Audience { members, announce_issuer }`
(`mush-economy/src/record.rs:348-364`), whose `announce_issuer` default of `false` is already T6's
"notice of the window, not the plan"; consequences write `ControlRecord` and `Front` changes through
`mush-faction` (`data.rs:465-486`, `sim.rs:53-98`). Every formation node names exactly one owning
`OrgScopeId` and `force.*` capabilities resolve through the existing grants — the vocabulary is
composition-root data, not engine vocabulary (`mush-org/src/wiring.rs:78-93`) — so R2 needs no core
change.

**5B — inside `mush-faction`.** Worse: `mush-faction` has no direct economy edge (it resolves an org
and calls through, `econ.rs:137`), while a declared operation needs escrow, audience visibility and
obligations first-hand; and §14's crate boundary lists the campaign/warfare layer separately from
faction. It would also make `mush-faction` the home of force composition, which T8 explicitly keeps
out of any organization hierarchy.

**5C — in `ruleset-tacsim`.** Worse: fronts, control, bonds and audience visibility are game-neutral —
a WoD chronicle can declare an operation against a nightclub. §14 puts only propulsion, resources,
recipes, formulas and policy in the ruleset.

### 6. `ruleset-tacsim` configuration and the corridor (§16 item 6)

**6A — everything expressible as a table is a versioned TOML content pack, loaded like E6's catalog
packs (recommended).** Data: propulsion and route tables, the ten commodity families, recipes,
formation kinds (Q9), insurance and warfare policy, and the whole Mars–Phobos–Ceres corridor (§11) as
its own pack. Code: only what a table cannot express — the combat pulse resolver, availability and
skill tests, and the drivers a definition names. Packs carry stable ids and definition versions and
are source-controlled (08e answer 10); `[tacsim.*]` host config carries deployment switches and pack
paths only, mirroring `[shadowrun.economy].catalog_paths` (08e answer 16). This fits what the engine
already enforces: `DomainRecord.driver` is an exact `DefinitionRef` with owner/kind/id/version/digest
identity (`mush-sim/src/data.rs:227-232`), and K3 validates exact descriptors at registration
(`mush-tactics/src/module.rs:622-634`) — a pack is content behind a versioned definition, not a config
free-for-all.

**6B — hard-code the corridor in Rust for the first playable.** Worse: §11 frames the corridor as the
*proving* content — if it is code, tuning the loop that decides whether the game is fun costs a
rebuild and a review, and the second region is a copy-paste. It also re-imports the price and route
literals that 08e answer 10 just moved out of the crates.

**6C — staff-authored in-world content.** Worse: no diff, no review, no version pin; a definition
digest that shifts under a live contract breaks T1's frozen route assumptions.

### 7. Recovery provider (§16 item 7, T7)

**7A — a funded NPC/government program on an obligation + lease template (recommended).** No new
engine machinery: a registered system principal with a real, **capped** account (doc 10 7A's funding rule)
owns a small loaner pool; eligibility is a deterministic predicate over recorded facts (assets below a
threshold, no undischarged lease, no open fraud dispute) so staff are never the gate; the grant is a
contract whose deliverable places `Encumbrance::Lease { lessor, contract }` on the loaner
(`mush-economy/src/record.rs:530-537`), whose upkeep is an `Obligation` on the campaign cadence
(`obligation.rs:228-271`), and whose operating restrictions are ordinary conditions (no sale, no
transfer, no uninsured war-zone deployment). Upgrade terms: buy out at a declared price, or return and
re-lease at a higher tier; loss inside the restrictions is written off, loss outside becomes a debt.
`[tacsim.recovery]` config carries pool size, tiers, cap and thresholds. Grounded in T7 ("a lease
encumbrance is part of the floor, not the feature") and doc 10 §15 ("leases are the settlement
primitive").

**7B — a per-player entitlement that mints a free hull.** Worse: an uncapped provider is exactly the
uncontrolled faucet doc 10 7A forbids, and doc 10 3A's "meaningful loss" evaporates when replacement is free.

**7C — employment and crew seats only, no equipment.** Under-delivers: doc 10 3A names "employment, crew
positions, leased equipment **or loaners**" and T7 names an inventory or production source for loaner
units; seats alone strand a bankrupt owner-operator away from the thing they actually play.

### 8. Time compression (§16 item 8)

**8A — one campaign cadence per world with a declared ratio and scene time floors (recommended; doc 10
6A).** `tacsim.campaign` is a `CadenceClock { source: RealUtc, period, catch_up_cap, position, … }`
(`mush-economy/src/clock.rs:229-249`); doc 10 1A's "≈ one OOC hour per IC day" is expressed as `period`
seconds per campaign period plus a declared `ic_seconds_per_period` used **only for rendering IC
dates** — a named cadence, never a literal ratio in code or in a record. Journeys, doc 10 T1 alignment bands
and per-sortie maintenance ride `Manual` cadences the sim advances at boundaries
(`advance_cadence_manual`, `clock.rs:300`; doc 10 §15 notes the hook from the sim does not yet exist).
WoD's and Shadowrun's clocks stay separate `CadenceId`s and never derive from the campaign clock, so a
TacSim compression change cannot re-price a Shadowrun lifestyle. `TimeJump` forgiveness is already the
cadence's `catch_up_cap` with the excess recorded as forgiven (`clock.rs:264-279`); E7 sets it per
clock and adds the **scene IC time floor** doc 10 6A requires, since `scene_ic_time` is start + rounds today
with nothing stopping a post-arrival event from being stamped before arrival
(`mush-scenes/src/data.rs:171-178`).

**8B — a global OOC:IC ratio the server multiplies.** Worse: E4 already refused it — a schedule
expresses `next_due` as a period index "never on an IC:OOC ratio or a literal calendar"
(`clock.rs:219-225`) — because one ratio change silently re-prices every outstanding obligation in
every ruleset at once.

**8C — per-system ratios (journeys, payroll, fronts).** Worse: §20 exists to stop composition
multiplying clocks; three ratios make "does this arrive before payday" unanswerable.

### 9. Formation kinds and echelons as data (§16 item 11, T8)

**9A — generic engine record, ruleset-authored instances, composition advisory by default
(recommended).** The engine stores `FormationKind { id, branch, echelon, composition: Exact | Min |
Max | Preferred, command_roles }` — T8's own field list — and `ruleset-tacsim` authors the instances in
the Q6 packs. Composition is **advisory at task-force assembly** and enforced only where the ruleset
marks it `Exact`. T8's own example demands this: a four-unit lance and a five-unit star "may share an
echelon but remain distinct kinds", which is only expressible if the engine never learns what a lance
is. Advisory because a task force is operation-scoped and lossy by design (detached subformations,
individual units, support assets) — a battalion that lost a company must still deploy.

**9B — engine-enumerated echelons with fixed sizes.** Worse: T8 forbids hard-coding military
vocabulary, and a mixed ground/aerospace/naval world would need an engine change per branch.

**9C — composition strictly enforced.** Worse: every casualty becomes an administrative refusal, and
R1's "a unit is in at most one operation at a time" already carries the integrity constraint that
actually matters.

### 10. Human-directed NPC seats (§16 item 12, T10–T13)

**10A — the invitation is literally an E4b2 contract with `Audience` visibility (recommended; doc 10
R6).** Every field T10 wants frozen is already a consent-surface field: parties (commander, player),
terms (compensation, if any), conditions (required certification or standing), deadline (the window),
visibility (a public seat vs a covert operation's allowlist), and the adjudicator/evidence pair for
disputes (`mush-economy/src/contract.rs:450-480`; `record.rs:348-364`). Acceptance freezes the surface
by hash exactly as a roster lock does, and revocation is a cancellation. T11's guardrails are **not**
contract prose: they are the granted channel set on the NPC's `StationSpec`
(`mush-sim/src/data.rs:414-435`) plus a control-epoch bump at handoff (`data.rs:615-622`), so a stale
intent from a revoked proxy fails the epoch check. The muster (T14) still uses the party subsystem
(08e answer 21) — the seat contract references a party snapshot; it does not replace it.

**10B — a lighter party-subsystem record.** Worse: a party record has no consent hash, no frozen
terms, no audience class and no named adjudicator, so T10's "everything freezes in the accepted
invitation" would be rebuilt inside it; and T13's participation-indexed settlement wants the same
record to index reward, which outcome-indexed terms already do (`contract.rs:235-256`).

**10C — a staff-granted lease with no consent record.** Worse: T10 requires the player's consent
("they cannot be forcibly attached") and T11 requires dual attribution; a lease records neither.

### 11. Account-distinctness oracle (§16 item 13, R3/R7)

**11A — a boolean oracle in `mush-core`, over character ids, with a staff-only audit verb
(recommended).** Signature: `fn accounts_distinct(&self, w: &World, characters: &[ObjId]) ->
Option<bool>` on a wiring-frozen provider seam shaped like `FactProvider`/`CustodyProvider`
(`mush-economy/src/facts.rs:79-93`), where `None` = unknown and the calling policy **fails closed**.
One staff verb (`@account/audit`) answers "same account as X?" and never prints an identity (R7).
Core rather than `mush-org` because the callers — campaign roster lock, side assignment, the party
subsystem, contract acceptance — should not each gain an org dependency, and `mush-org` is about
authority *within an organization*; an account has no org.

**Flagged: the tree does not yet have what R3 assumes.** `AccountRec` is keyed by account **name**
(`account:{lowername}`) and holds `characters: Vec<CharacterRef>` (`mush-store/src/types.rs:718-741`);
there is no immutable account id, no character→account reverse index, and the record lives in the
store behind an async API with no world-side reader — the character *is* the login
(`mush-core/src/obj.rs:90-91`) and the only in-process trace is the session's `account: String`
(`mush-session/src/manager.rs:135-136`). So 11A carries two small prerequisites: **(i)** mint an
immutable opaque account id on `AccountRec` (stable across a rename), and **(ii)** hydrate a
character → account-id map at boot so the oracle answers synchronously inside a world command.
Neither puts the id into a journaled, contract or canon record.

**11B — in `mush-org`.** Worse: every consumer takes an org dependency for a fact about players, and
`mush-org`'s records are all org-scoped (`mush-org/src/record.rs:30-124`).

**11C — expose the account id to policy code.** Forbidden by R3/R7: never a principal, never in a
journaled or canon record. A boolean cannot leak what a shared id would.

### 12. Tactical intents and the K action lane (doc 09 C2/K2; K5–K6 planned)

**12A — tactical intents enter through the K2 coordinator lane from the first pulse (recommended).**
Doc 09 classifies new authoritative state-family actions as prepared-path work, and K3 already proves
the shape with the `brace` provider (`mush-tactics/src/module.rs:539-635`): a typed `stance` family,
early idempotency, guard revalidation, exact-bundle durability, restart hydration. E7b's pulse
resolver then produces intents the same way, and doc 09 K5 (legal actions) and K6 (tactical actions)
land on a lane E7 is already on rather than one it must migrate to. Cost: E7b coordinates with the K
work in flight (K2 stragglers, the gateway custody protocol) — the plan names the exact contract
surface E7 consumes and freezes it.

**12B — legacy handlers now, migrate at K6.** Worse: doc 09 §1 calls the legacy lane a compatibility
risk until migrated, and a pulse-rate domain is the heaviest possible consumer to migrate later; the
`focus/brace/guarded` contract tests would be duplicated once per lane.

**12C — a TacSim-private action path.** Forbidden by doc 09 C2 (one prepared path) and by T2 (combat
RNG and intents live in the mechanical action journal, which is the K lane's journal).

## 4. Proposed E7 cut structure

Five cuts, each independently CI-green and adversarially verified before the next builds on it
(08a's rhythm). Confirm or reorder.

| Cut | Crate(s) | One line | Verify axis | Depends on |
|---|---|---|---|---|
| **E7a** | `mush-industry` (pure, then module) | deposits + survey bands + depletion; claims/licences as entitlements; recipes; lots with immutable lineage and the fungible↔lot bridge; the shape-conformant C8 capacity reservation; work orders as scheduled journaled production | reservation double-spend, lineage loss, depletion and yield arithmetic, restart mid-work-order | E4b2 + E4c (shipped); **no K item** (Q3 interim) |
| **E7b** | `mush-propagation` + `mush-sim` | **the tactical domain tick** (`DomainClock::Pulse`; decision 2A — the tree has only `Passive`, `mush-sim/src/data.rs:210-241`); contacts and order envelopes on one reachability step (§16 items 4 + 14); carrier journeys — the carrier root travels with its interiors, occupants and cargo coherent (item 9); actor-side active-station focus, reach/traversal/safe-boundary handoff and republished links (item 10) | fog-of-war leaks, order supersession under ECM, carrier topology coherence across a leg, one-focus enforcement | nothing beyond the tree; `ChannelGrant` (doc 09 C8) folded here |
| **E7c** | `mush-campaign` | one record = declared operation + task force (R1); formation trees bound to org scopes (R2); bond as an economy hold; freeze-at-activation; consequences into fronts and control (items 11, 13) | freeze-at-activation, one-operation-per-unit, audience/visibility leaks, front consequence idempotency | E7b (an operation needs delivered orders and contacts), `mush-faction`, E4b2 |
| **E7d** | `mush-campaign` + party + `mush-sim` | human-directed NPC seats as audience contracts with dual attribution and deterministic fallback; crew complements and readiness obligations; scheduled musters and open crew calls; participation facts (items 12, 15; T9–T16) | proxy guardrail escapes, revocation at a safe boundary, absent-player settlement, one foreground focus | E7c; the Q11 account oracle; the party subsystem (E6b's, or E7d ships it) |
| **E7e** | `ruleset-tacsim` + `mushd` config | propulsion/route tables, the ten commodities, recipes, combat formulas, insurance and warfare policy; the Mars–Phobos–Ceres corridor pack; the recovery provider | the §11 proving loop end to end; content-pack versioning and digest stability under live contracts | E7a–E7d; **E6c** for the market (see below) |

**Dependencies to confirm.** (1) **E6c (`mush-market`, gated on the safe market projection) gates the corridor's "one open
market"** — E7 must not build a second one (08e answer 20). Recommended: the corridor ships with
**direct vendor sale** (E6b's vendor-from-stock path) and the order book lands when E6c does; say if
you would rather E7e wait. (2) **No E7 cut blocks on a K item.**
Q3's interim is deliberately K-independent, and the sim-side staleness check is already the control
epoch (`mush-sim/src/data.rs:635-640`); only the *migration* of the industry reservation waits on 08c
§12's three K3 items. (3) **The proving story's shortest path** is E7a → E7b → a partial E7e (corridor
content, no recovery provider) → E7c → E7d: extraction and manufacture come from E7a, hauling and
interception from E7b, the control change from E7c, and multicrew/staffing from E7d. If you would
rather see a playable corridor sooner than a correct campaign layer, say so and E7c/E7d swap with a
stub operation record.

## 5. What E7 hands back to E5/E6

- **Fact kinds `mush-sim` and `mush-tactics` register** on E4c's `FactProvider` (`facts.rs:62-77`,
  wired at `mush-server/src/modules.rs:148`): `sim.journey.arrived`, `sim.journey.failed`,
  `sim.station.served` (R6/T13's participation fact — doc 10 §15 already names its shape
  `{ actor, station, from, to }`), `sim.cargo.delivered`, `sim.docking.completed`,
  `tactics.sortie.started` / `.ended`, `tactics.unit.disabled` / `.abandoned` / `.captured` /
  `.destroyed`, `tactics.salvage.recovered`. Each provider owns a stable id space per domain journal;
  the economy points *at* that journal and never duplicates it (T2).
- **`scene.outcome` stays E6b's.** The canon-outcome table is composition-root data and the server
  registers nothing until E6b ships it under facilitator authority
  (`mush-economy/src/contract.rs:311-322`). E7 consumes it; E7 must not register a scene outcome kind
  of its own.
- **The market projection stays E6c's.** `mush-market` owns listings, order books, quotes and the safe
  public projection (08e answer 20; doc 10 §14). E7 supplies **inventory locality** — a lot or a
  commodity balance is held at a station or facility — and consumes quotes; it never builds an order
  book.
- **Two small upstream asks E7 depends on**, both already noted in doc 10 §15: the **scene IC time
  floor** in `mush-scenes` (doc 10 6A), and the **manual-cadence advance hook from the sim**
  (`advance_cadence_manual` exists; the caller does not).
