# 10a — TacSim E7: owner questions

**Answered 2026-09-03: the owner accepted every recommended option (1A 2D 3A 4A 5D 6A 7A 8D 9A 10D 11A 12A 13A–22A) and the doc 10 R6 amendment (doc 10 is now Draft 0.4); all binding.**

**Draft 0.2 — 2026-09-03 (Draft 0.1 + an external review, adjudicated; §6).** The survey the owner answers before the E7
implementation plan is written, in the shape of [08e](08e-e6-owner-questions.md): numbered questions, lettered options
with a **recommended** default, and the code fact each recommendation rests on. Answer `1A, 2D, …`, or `defaults` to
take every recommendation. Grounded in the tree at `34a53c0` (main, K3 merged); every `file:line` was read.

---

## 1. Purpose and what is already decided

Doc [10](10-tacsim-foundations.md) is binding and this survey does **not** re-ask any of it: the foundational decisions
**1A–7A** (§12) and **C1A–C30A** (§18), the addenda **T1–T16** (§13, §19), the sharpenings **R1–R7** (§21), the crate
boundary (§14), the E4–E6 implications (§15) and the environmental seam (§20) are settled and cited as givens. What
remains open is doc 10 §16: **items 1–8 are definition decisions** with no answer yet, and **items 9–15 are accepted
requirements** needing decomposition into build cuts. This document asks **twenty-two** questions — one per §16
definition item, four that §16 and §21 force but doc 10 never decides, and ten the external review surfaced as decisions
hidden inside the first twelve (§6) — and proposes a cut structure (§4) and the handback to the economy programme (§5).
Where an option contradicts a locked decision it is named as such and rejected, not re-litigated. One question (Q10)
proposes an **amendment to doc 10 R6**, flagged for the owner rather than assumed.

## 2. Grounding: what the tree has today

- **Journeys are plain-object routes** whose traveler must be **un-placed** — a domain, interior, mount or station host
  is refused (`mush-sim/src/route.rs:67-90`, `commands.rs:1529-1563`).
- **Station assignments are host-keyed** (`HostAssignments` over `StationAssignment`, `mush-sim/src/data.rs:446-462`);
  the derived index is actor → *set* of units (`:502-503`) with **no active focus**, `StationSpec` grants an explicit
  channel set (`:414-435`), and links are a pure projection taking max priority across an actor's seats
  (`links.rs:40-55`) — the hazard doc 09 C8 answers with `ChannelGrant` (`09:989-991`). **Control epochs** are per-unit
  and monotonic (`data.rs:529-533`, `608-640`); `DomainClock::Passive` is the only mode, so nothing ticks tactically.
- **Custody is a provider table over *objects*** — `handles`/`claims`/`move_into`/`move_out`, exactly one claiming
  provider, validate-then-mutate (`mush-economy/src/custody.rs:13-20`, `37`, `66`, `114-144`), with
  `InteriorCustodyProvider` the one registered endpoint (`mush-sim/src/custody.rs:51`; `modules.rs:139`).
- **Fronts, control kinds, contracts and cadence clocks are all built**: `Front`/`Clock` flipping `(RegionId,
  ControlKind)` and a per-region `ControlRecord` (`mush-faction/src/sim.rs:53-98`, `data.rs:420-486`, `econ.rs:137`);
  outcome tables, four distinct contract roles, a `ConsentSurface`, `EconomicVisibility::Audience` and
  `Encumbrance::{Lease, Disputed, Collateral}` (`mush-economy/src/contract.rs:235-276`, `450-480`, `530-536`;
  `record.rs:348-364`, `510-543`); `CadenceClock` with `catch_up_cap` and `advance_cadence_manual`, whose `next_due` is
  a **period index**, "never on an IC:OOC ratio or a literal calendar" (`obligation.rs:228-271`, `clock.rs:208-249`,
  `219-225`, `300`). **Scene time is start + rounds with no floor** (`mush-scenes/src/data.rs:171-178`).
- **The fact seam is already shaped for sim/tactics** — `ExternalRef` resolved by `FactProvider::resolve` over a
  wiring-frozen table where unknown = unsatisfied, only `mush-jobs` registering (`record.rs:307-316`, `facts.rs:13-33`,
  `62-109`; `modules.rs:148`); canon outcome kinds are producer-registered, none yet.
- **The C8-shaped reservation in code is a guard, not a store**: `AuthorityGuard::Reservation`, coordinator-created,
  routed to a per-`(owner, kind)` wiring-registered validator (`mush-core/src/action.rs:1036-1064`, `1420-1445`;
  `module.rs:1792-1795`). No `ReservationRecord` exists: doc 09's record (`09:997-1009`) is vocabulary.
- **The authoritative bundle is hard-capped**: replay writes 1_024, definitions 256, durable-outbox intents 256, payload
  256 KiB, bundle 16 MiB (`mush-core/src/authoritative.rs:28-38`). **Fungibles are an economy primitive**
  (`InstrumentKind::{Currency, Commodity, Requisition, Entitlement}`, `scale` 0 = indivisible, `record.rs:28-61`) and
  **contact vocabulary exists with no producer** (`ContactId`, `PerceivedRef::Contact`, `action.rs:142-147`, `656-669`).
- **The account layer is store-side and name-keyed** — `AccountRec` at `account:{lowername}` holds `characters:
  Vec<CharacterRef>` (`mush-store/src/types.rs:718-741`), yet a character *is* the login (`mush-core/src/obj.rs:90-91`);
  sessions fan out to **many connections** and go linkdead on a driver-timed grace (`SessionConfig::linkdead_grace`,
  default 300 s, `mush-session/src/lib.rs:57-72`; `manager.rs:44-46`).
- **Absent entirely** (repo-wide grep of `crates/*/src`): `Formation`, `TaskForce`, `TacticalOrder`, deposit, recipe,
  lot, `mush-industry`, `mush-inventory`, `mush-market`, `mush-party`, `mush-propagation`, `ruleset-tacsim`.
  `mush-party` is **E6a's** new neutral crate, `{core}` deps only (08f §2, `08f:90`, `:94-95`).

## 3. The questions

### 1. Extraction and yield model (§16 item 1)

**1A — deterministic yield curve per deposit, with depletion (recommended).** A `Deposit { id, region, commodity, grade,
accessibility, reserve, curve, surveyed_bands }`; a survey reveals *bands* (grade class, reserve class), never the exact
reserve, and a revealed band is a recorded fact. Production is **scheduled, journaled lot creation** on a deterministic
key — doc 09 C8 already lists production and upkeep among the bounded-idempotency-key actions (`09:1019-1028`).
Variation comes from **discrete campaign events** (accessibility, equipment condition, collapse, licence suspension)
moving effective yield, not per-period rolls. **Tenure is its own record, not a balance:** a claim needs `ClaimRecord {
deposit_or_area, issuer, holder, exclusivity, priority, term, transferability, contest_state }`; a licence is an
`InstrumentKind::Entitlement` account issued by a government principal (`record.rs:52-61`) proving *authorization* to
survey, extract or export — not tenure. See **Q14**.

**1B — stochastic yield seeded per period.** Legal, rejected on legibility. T1 freezes a contract's route assumptions
and terms at publication; it does **not** forbid stochastic extraction, and seeded randomness in the mechanical journal
(T2) would satisfy both. 1A wins because **industrial planning must be legible**, and doc 10 4A supplies variation as
heterogeneity between deposits ("yield, quality, accessibility, location") plus events. **1C — abstract infinite
deposits with licence fees.** Rejected by doc 10 4A and §5 (conflict and comparative advantage need scarcity), and by 7A
generalized: an infinite deposit is an uncontrolled faucet.

### 2. Lot representation (§16 item 2)

**2D — a neutral inventory authority owns location-bound quantities *and* lots; the ruleset decides representation per
commodity (recommended).** One layer (Q13) owns **fungible pools** — a quantity held at a place — and **lineage-bearing
lots**, with split, merge, transfer and custody reservation as its verbs, every balance bound to a **custody endpoint**
(vault, facility, cargo hold, container). That endpoint is the **existing E3 custody model extended from objects to
quantities and lots**, not a parallel custody system (same table, same one-claiming-provider and validate-then-mutate
rules, `mush-economy/src/custody.rs:66`, `114-144`; `InteriorCustodyProvider` is already the shipboard endpoint).
Economy `InstrumentKind::Commodity` accounts still give double-entry **value** settlement (`record.rs:38-39`), never
locationless cargo, and a money hold is never the physical reservation (T4). Interconversion is one journaled,
attributed act each way: `dissolve(lot)` creates or updates an immutable **pool-tranche lineage root** and credits the
pool; `crystallize(pool, n)` mints a lot whose parent **is that tranche** — lineage is never discarded, only coarsened
in the active projection (T5 exactly). Lot fields are `{ id, commodity, quantity, grade, lineage }`; **not** `legal` and
**not** `custody`. Custody belongs to the endpoint, so a lot has one location and one custodian; stolen or disputed
status follows the **title** record (`Encumbrance::Disputed`, `record.rs:522-527`; E6b's `report_stolen`); embargo
legality is a **jurisdiction/policy test at transaction time**, not a field travelling with matter — the same cargo is
legal one hop away; licensing is the **acting principal's** entitlement (Q1/Q14).

**2A — instruments for fungibles, lots for the rest, nothing else. Rejected.** An account has no location, and doc 10 §5
says orders belong to a station and "buying remotely does not teleport goods"; money holds are not inventory
reservations either (T4). (Draft 0.1 recommended this; §6.) **2B — everything is a lot.** Worse: every propellant top-up
becomes a lineage write, with no gain where provenance is irrelevant (T5's own carve-out). **2C — everything fungible
with a provenance tag.** Worse: a tag is not lineage — T5's "never discarded" is unsatisfiable once two tagged parcels
merge.

### 3. C8 interim (§16 item 3)

**3A — `mush-industry` ships a reservation record shape-conformant to C8, with a named migration trigger
(recommended).** 08c §12's conformance strategy applied a second time. The record carries doc 09 C8's exact fields
including its `Held | Consumed(commit) | Released | Expired` states (`09:997-1009`), lives in `mush-industry`'s own
documents, and registers an `AuthorityGuardKind::Reservation` validator under its own `OwnerId`
(`mush-core/src/action.rs:1060-1064`; `module.rs:1792-1795`) so a K3 action can already name one — the pattern is
tactics' `brace` provider (`mush-tactics/src/module.rs:76`, `539-556`, `600-635`). **"Consume atomically" before K3's
mixed write sets** means one plan, one batch, one journal-authoritative work-order document naming every reservation it
needs, a single companion-carrying terminal batch, no fallible step after the first install — E4b2's plan-first rule,
the strongest atomicity the tree offers. **The trigger opens a review, never an automatic migration.** When 08c §12's
three K3 items — **mixed write sets**, a **`Ctx` family read path**, **the seal folded into the bundle** — all land, a
**migration review** opens; 08c §12 says of the equivalent move for `obligation:{id}`/`contract:{id}` that it "is then
still an explicit **owner decision**, not a mechanical port".

**3B — wait for the kernel coordinator.** Worse: 08c §12's K3 review names three blockers with no owner and no date,
while §11's proving story needs a refinery slot and a drydock reservation immediately. **3C — reuse the economy
`Hold`.** Forbidden by T4 and doc 10 §15: "Holds stay money-only … must not grow a resource field."

### 4. Propagation layer (§16 items 4 + 14, R4)

**4A — one `mush-propagation` crate owning contacts *and* delivery envelopes over a shared physical propagation context,
with separate resolvers (recommended).** Minimum deterministic model, no belief engine: per-domain `ContactRecord {
observer, subject, first_seen, last_seen, confidence_band, revealed_attributes, generation }` keyed by the kernel's
existing `ContactId` and projected as `PerceivedRef::Contact { provider, id, generation }` (`action.rs:142-147`,
`656-669`) — core has the vocabulary, only a producer is missing. `Envelope { id, issuer, recipient, payload_ref,
revision, state: Queued | Transmitted | Delivered | Acknowledged | Expired | Superseded, path }` carries T15's order
lifecycle and C9A's consequential communications. **One context, two resolvers — not one reachability step.** They share
geometry, medium, range, occlusion, interference and time-of-flight, and answer different questions: **detection**
resolves signature, emissions posture, medium, line of sight, sensor capability, noise floor and observer posture;
**delivery** resolves addressing, transmitter/receiver compatibility, routing, relays, encryption, jamming and
store-and-forward. A jammed unit that still takes orders and a courier intercepted but never detected are R4's two
cases. Cadence: tactical propagation runs on the **domain tick**; with none active, interplanetary mail, market reports
and couriers run a **coarse scheduled delivery** pass on a campaign cadence. An **acknowledgement is itself a delivery**
that must reach the issuer — T15's commander sees a replacement unacknowledged only because the ack had to travel.

**4B — contacts in `mush-sim`, delivery in a separate order subsystem.** Worse: R4 was accepted because the two must
share one substrate, and this duplicates the propagation context on the hot path. **4C — both inside `mush-sim`.**
Defensible and cheaper — domains and the tick already live there (`data.rs:210-241`) — but `mush-sim` is already the
largest sim crate and propagation has consumers with no spatial dependency (doc 10 §18.2). Take 4C if you prefer fewer
crates to a clean consumer boundary.

### 5. Campaign/warfare layer ownership (§16 item 5, R1/R2)

**5D — a new neutral `mush-campaign` crate; one logical operation, separately versioned terms and runtime under one
`OperationId` (recommended).** R1 is satisfied by the shared id and the one-to-one link, without conflating consent with
runtime. **`OperationTerms`** freezes at activation (T6): objective, legal targets, stakes and consequences, bond,
eligible sides and defenders, notice and vulnerability window, and a consent-surface hash reusing E4b2's helper.
**`OperationRuntime`** is mutable: participants, operational parentage, reinforcements and withdrawals, casualties,
station handoffs, contact and order state, progress and outcome facts. **Changing the roster never changes the terms
hash** — a reinforcement is not an amendment to what the defenders consented to be told. The bond is an economy hold
placed by the attacker's own authority; `EconomicVisibility::Audience`'s `announce_issuer` default of `false` is already
T6's "notice of the window, not the plan"; consequences write `ControlRecord` and `Front` changes through
`mush-faction`. Every formation node names exactly one owning `OrgScopeId` and `force.*` capabilities resolve through
the existing grants (`mush-org/src/wiring.rs:78-93`), so R2 needs no core change. Settlement is **Q17**.

**5A — a single mutable record that is both operation and task force. Rejected.** It re-learns the E4b2 lesson the hard
way: every roster edit becomes either a terms amendment or a hole in the hash. (Draft 0.1 recommended it; §6.)

**5B — inside `mush-faction`.** Worse: it has no direct economy edge (`econ.rs:137`) while a declared operation needs
escrow, audience visibility and obligations first-hand, §14 lists the campaign layer separately, and it would make
`mush-faction` the home of force composition, which T8 keeps out of any org hierarchy. **5C — in `ruleset-tacsim`.**
Worse: fronts, control, bonds and audience visibility are game-neutral, and §14 puts only propulsion, resources,
recipes, formulas and policy in the ruleset.

### 6. `ruleset-tacsim` configuration and the corridor (§16 item 6)

**6A — everything expressible as a table is a versioned TOML content pack, in two kinds (recommended).**
**Rules-definition packs** are immutable and referenced by digest: propulsion and route tables, the ten commodity
families, component definitions, recipes, formation kinds (Q9), combat and insurance policy. **Campaign-seed packs**
instantiate mutable world state **once**: corridor locations, organizations, initial deposits, facilities, markets,
inventories and NPCs. The distinction matters — a rules pack is re-pinned and diffed, while re-running a seed pack would
duplicate a mine. Code: only what a table cannot express — the combat pulse resolver, availability and skill tests, the
drivers a definition names. `[tacsim.*]` host config carries deployment switches and pack paths only, mirroring
`[shadowrun.economy].catalog_paths` (08e answer 16). The engine already enforces the shape: `DomainRecord.driver` is an
exact digest-bearing `DefinitionRef` and K3 validates exact descriptors at registration (Q18).

**6B — hard-code the corridor in Rust for the first playable.** Worse: §11 frames the corridor as the *proving* content,
and if it is code, tuning the loop that decides whether the game is fun costs a rebuild and a review. **6C —
staff-authored in-world content.** Worse: no diff, no review, no version pin, and a definition digest shifting under a
live contract breaks T1.

### 7. Recovery provider (§16 item 7, T7)

**7A — a funded organization running an obligation + lease template (recommended).** No new engine machinery. The
provider is normally a **government or corporate organization principal** owning the loaner inventory, accounts,
contracts and policy; a system faucet may **subsidize** it under the `[economy]` mint cap via `create_system_obligation`
(08f §3.3), because a `System` principal can never itself be a contract party — 08f's D-E6-4, which the tree forces
(`08f:161`, `:213`, `:264`). Eligibility is a deterministic predicate over recorded facts examining **accessible
operational assets** (unencumbered, reachable, deployable) rather than total title, so a player whose only hull is an
unreachable wreck still qualifies; staff are never the gate. The grant is a contract placing `Encumbrance::Lease` on the
loaner (`record.rs:530-537`), with upkeep an `Obligation` on the campaign cadence and restrictions as ordinary
conditions (no sale, transfer or uninsured deployment). The plan must state **pool exhaustion** behaviour, queue
priority, per-player cooldown, insurance for approved operations, and how the pool **replenishes**. Upgrade terms: buy
out at a declared price or re-lease a tier up; loss inside the restrictions is written off, outside it becomes a debt.
`[tacsim.recovery]` carries pool size, tiers, cap and thresholds.

**7B — a per-player entitlement that mints a free hull.** Worse: an uncapped provider is the uncontrolled faucet doc 10
7A forbids, and 3A's "meaningful loss" evaporates when replacement is free. **7C — employment and crew seats only.**
Under-delivers: doc 10 3A names "leased equipment **or loaners**" and T7 an inventory or production source; seats alone
strand a bankrupt owner-operator away from the thing they play.

### 8. Time compression (§16 item 8)

**8D — one versioned world timeline owned by the kernel, with derived cadence adapters (recommended).** Doc 09 §2.5
already names it: `ClockRef::WorldIc { generation }`. The timeline is versioned epochs `{ real_anchor, ic_anchor, rate,
generation }`; a rate change **opens a new generation** and never reinterprets earlier facts — which is why doc 09
compares two stamps only when their complete clock identities match (`09:296-316`). The **campaign layer sets rate
policy** (doc 10 1A's ≈ one OOC hour per IC day is a policy value, not a constant in code); every subsystem registers a
**derived cadence adapter** over it — economy cadences, industry cycles, civil schedules, fronts, journeys — and 08c §12
already calls `EconomicClock` a **pre-K4 adapter shape** that becomes a thin alias over `ClockRef` at K4. **Multiple
named cadences are fine; multiple incompatible real→IC mappings are the danger** — WoD's and Shadowrun's clocks stay
separate `CadenceId`s and never derive TacSim's rate, so a compression change cannot re-price a Shadowrun lifestyle.
`catch_up_cap` stays **obligation-sweep policy** (`clock.rs:264-279`); E7 must give **journeys, production, fronts and
tactical domains each an explicit catch-up policy** — they are not obligations. Scenes enforce the timeline's **IC time
floor** (doc 10 6A), which `scene_ic_time` lacks today.

**8A — the economy's campaign cadence is the world clock. Rejected.** The economy is a *consumer* of time, not its
authority: journeys, domain ticks, civil phases (§20) and fronts need the same mapping, and none should reach into
`mush-economy` to ask what time it is. (Draft 0.1 recommended this; §6.)

**8B — a global OOC:IC ratio the server multiplies.** Worse: E4 already refused it — `next_due` is a period index "never
on an IC:OOC ratio or a literal calendar" (`clock.rs:219-225`) — because one ratio change silently re-prices every
outstanding obligation in every ruleset. **8C — per-system ratios (journeys, payroll, fronts).** Worse: §20 exists to
stop composition multiplying clocks, and three *mappings* make "does this arrive before payday" unanswerable — three
*cadences* over one mapping are fine, and that is 8D.

### 9. Formation kinds and echelons as data (§16 item 11, T8)

**9A — generic engine record, ruleset-authored instances, composition as a constraint set (recommended).** The engine
stores `FormationKind { id, branch, echelon, composition, command_roles }` where **`composition` is a constraint set**,
not one enum: `minimum`, `maximum`, `preferred`, allowed member categories, required command and support roles — with
**`exact` expressed as `minimum == maximum`**, not a separate mode. T8's "exact/minimum/maximum/preferred" is four knobs
on one shape, and `ruleset-tacsim` authors the instances in the Q6 rules packs. **Strictness is scoped**: they apply at
*creation*, at *doctrine qualification* ("is this still a full squadron?"), or as *an operation's requirement* — never
to a formation's existence or its ability to deploy. An understrength permanent formation is normal, not an error. T8's
own example needs it: a four-unit lance and a five-unit star "may share an echelon but remain distinct kinds".

**9B — engine-enumerated echelons with fixed sizes.** Worse: T8 forbids hard-coding military vocabulary, and a mixed
ground/aerospace/naval world would need an engine change per branch. **9C — composition strictly enforced on existence
and deployment.** Worse: every casualty becomes an administrative refusal, and R1's "at most one operation at a time"
already carries the real constraint.

### 10. Human-directed NPC seats (§16 item 12, T10–T13)

**10D — a `SeatOffer`/`SeatGrant` campaign record carrying the contract's consent-hash discipline; an E4 contract only
where a fictional principal has terms (recommended).** The governing fact: **an OOC proxy player is not an in-world
principal.** C1A/R3 forbid an account principal and keep it out of any contract or canon record, and T13 says the absent
character did not take the station — so an E4 contract can name *neither*, and there is no third candidate. The offer is
therefore a **campaign/control record** carrying **the same canonical consent-hash discipline** as E4b2's surface, by
**sharing the hashing helper, not forking it**: frozen fields are audience and eligibility, expiry, station scope (unit,
station, channels, mission order, ROE, permitted area), **decision-driver attribution as an opaque driver token** (T9),
and revocation and fallback terms. The **account reference is audit-only** (R7) and lives outside the grant — **Q19**.
Live control is a **lease derived from the grant**, not the grant itself: revocation bumps the unit's control epoch
immediately (`mush-sim/src/data.rs:615-622`) so stale intents fail the epoch check, and the seat returns to its declared
fallback at a safe boundary (T11). T11's guardrails are the granted channel set on the NPC's `StationSpec`
(`data.rs:414-435`), not contract prose. An **E4 contract links to the operation only when an actual fictional
principal** — embodied character, named NPC, organization — has economic terms. An unpaid proxy seat needs none, and
inventing one to hold the consent hash is 10A's error. The muster (T14) uses Q21's party subsystem.

**Doc 10 amendment this forces — for the owner to accept into doc 10 Draft 0.4.** R6 currently makes T10 invitations
"templates … over E4b1/E4b2/E4c". Proposed replacement: *"T10 invitations are **consent grants that share the contract's
consent-hash discipline**; an E4 contract exists only where a fictional principal has economic terms."* The rest of R6
(C15A readiness, C18A passage, T13 settlement, matchmaking standing as policy input) is unchanged.

**10A — the invitation is literally an E4b2 contract with `Audience` visibility. Rejected.** Every frozen field maps
onto a consent surface, which is why Draft 0.1 recommended it — but the *party* question has no legal answer, and E4b2
contracts settle money between principals. (Doc 10 R6 as written assumes this option; see above.) **10B — a lighter
party-subsystem record.** Worse: a party record has no consent hash, no frozen terms, no audience class and no
revocation semantics, so T10's freeze would be rebuilt inside it. **10C — a staff-granted lease with no consent
record.** Worse: T10 requires the player's consent and T11 requires dual attribution; a bare lease records neither.

### 11. Account-distinctness oracle (§16 item 13, R3/R7)

**11A — a synchronous provider interface in `mush-core`, implemented by the server/store layer (recommended).** Core
defines the seam, shaped like `FactProvider`/`CustodyProvider` (`mush-economy/src/facts.rs:79-93`); the server/store
layer implements it. It offers `same_account(a, b) -> Option<bool>`, `all_accounts_distinct(characters)`, **atomic claim
and release of the account's foreground focus** (C21A/T12 — Q20), and **synchronous updates** when characters are
created, linked or removed and when accounts are merged or renamed. `None` = unknown and every calling policy **fails
closed**. One staff verb (`@account/audit`) answers "same account as X?" without printing an identity (R7). Core, not
`mush-org`, because the callers — roster lock, side assignment, parties, seat grants — should not each gain an org
dependency, and `mush-org` is authority *within an organization*; an account has no org.

**Flagged: the tree does not yet have what R3 assumes.** `AccountRec` is name-keyed with no immutable id and no
character→account reverse index, and lives in the store behind an async API with no world-side reader
(`mush-store/src/types.rs:718-741`; the character *is* the login, `mush-core/src/obj.rs:90-91`). So 11A carries two
prerequisites: **(i)** mint an immutable opaque account id on `AccountRec`, stable across a rename, and **(ii)** hydrate
a character → account-id map at boot so the oracle answers synchronously inside a world command. Neither puts the id in
a journaled or canon record.

**11B — in `mush-org`.** Worse: every consumer takes an org dependency for a fact about players, and `mush-org`'s
records are all org-scoped (`mush-org/src/record.rs:30-124`). **11C — expose the account id to policy code.** Forbidden
by R3/R7: never a principal, never in a journaled or canon record. A boolean cannot leak what an id would.

### 12. Tactical intents and the K action lane (doc 09 C2/K2; K5–K6 planned)

**12A — tactical *ingress* enters through the prepared K lane from the first pulse; the resolver commits once per domain
tick (recommended).** Two halves, and Draft 0.1 blurred them. **Ingress:** player commands and externally submitted
tactical intents are new authoritative state-family actions and take the prepared path — K3 already proves the shape
with the `brace` provider (`mush-tactics/src/module.rs:539-635`): typed family, early idempotency, guard revalidation,
exact-bundle durability, restart hydration. Doc 09 K5 (legal actions) and K6 (tactical actions) then land on a lane E7
is already on. **Resolution:** the pulse resolver does **not** create one K action per NPC, weapon or unit. It consumes
the frozen set of accepted intents for the tick and produces **one deterministic domain-tick commit per domain per
tick** — ordered deltas, RNG evidence, state hash — bounded by the K3 bundle limits (retained intent capacity,
per-record and per-bundle size, `authoritative.rs:28-38`), which is the real cap on a domain's active unit count and
belongs in the plan as a stated bound.

**12B — legacy handlers now, migrate at K6.** Worse: doc 09 §1 calls the legacy lane a compatibility risk, and a
pulse-rate domain is the heaviest consumer to migrate later. **12C — a TacSim-private action path.** Forbidden by doc 09
C2 (one prepared path) and T2 (combat RNG and intents live in the mechanical journal, the K lane's).

### 13. Inventory authority and locality (Q2's prerequisite)

**13A — a neutral `mush-inventory` layer, bound to custody endpoints, as the single source of quantity truth
(recommended).** It owns location-bound pools and lots, split/merge/transfer and custody reservations; industry,
markets, contracts and simulation *consume* it and none keeps a private stock number. Endpoints are the E3 custody model
extended from objects to quantities (`custody.rs:66`, `114-144`), so a cargo hold that moves with a ship needs no second
mechanism. **Decide this before Q2 and Q3** — a lot's fields and a capacity reservation's `resource` both depend on
where quantity lives.

**13B — `mush-industry` owns inventory.** Worse: market, contract deliverables and salvage need quantity truth without
an industry dependency, and doc 10 §14 keeps `mush-market` neutral. **13C — quantities live wherever their holder
lives.** Worse: three implementations of split/merge and no single answer to "what is actually here".

### 14. Claim tenure and conflict (Q1's prerequisite)

**14A — a unique claim record plus separate licence entitlements (recommended).** `ClaimRecord` (Q1's fields) is the
tenure object: **transferable**, **pledgeable** as collateral (`Encumbrance::Collateral`, `record.rs:510-543`),
**contestable** through a contract with a deadline and a named adjudicator (T3's shape), **suspendable** and
**seizable** by the issuing authority. Overlapping political authority is then representable without pretending an
entitlement balance is land title: two governments may each issue a claim over one area, and `ControlKind::{DeJure,
DeFacto}` (`mush-faction/src/data.rs:420-486`) says which is worth anything on the ground. Licences stay
`InstrumentKind::Entitlement` accounts with campaign-cadence expiry — authorization checked at transaction time.

**14B — claims are entitlement balances only.** Worse: a balance cannot be uniquely contested, carry priority, or be
pledged as a specific asset. **14C — claims are ordinary owned objects.** Defensible; but tenure needs term,
exclusivity, priority and contest state a plain object cannot hold, and title transfer alone cannot express suspension
by the issuer.

### 15. Tactical-domain activation (§16 item 9's other half)

**15A — deterministic promotion on stated triggers, demotion only at a quiet boundary (recommended).** **The governing
rule: the system never runs tactically as a whole.** A domain promotes to `DomainClock::Pulse` on four deterministic
triggers — a player character present in a contested context, a **credible contact** between opposed parties, an
**interception** along a journey leg, or the **activation of a declared operation** (Q5). It demotes only after a
**quiet boundary** (no contacts, no pending orders, no unresolved damage for a stated interval) and once a durable
summary is written. Hard limits: a maximum of concurrently active domains, with a stated policy at the limit (queue,
refuse with notice, or degrade to scheduled resolution). **Orders, contacts, damage and position are preserved across
promotion and demotion both ways** — demotion changes cadence, never state. Only `Passive` exists today
(`mush-sim/src/data.rs:210-241`).

**15B — every domain always ticks.** Worse: the corridor has hundreds of regions and one CPU. **15C — staff promote
domains manually.** Worse: an interception that only happens when staff are awake is not a simulation, and it violates
the standing "never park state on staff" rule.

### 16. Contact scope, fusion and false contacts (§16 item 4)

**16A — observations are local; transmitted reports create recipient knowledge (recommended).** A `ContactRecord`
belongs to a **platform or station**, never a side and never the world. Sharing is a **delivery** (Q4): a transmitted
report creates or updates the *recipient's* record with **source, age, confidence and provenance**, so a stale relayed
track is visibly stale and a false report is attributable — no omniscient fused record anywhere. **Unidentified**
contacts (no resolved subject) and **false** contacts (artefact, decoy, spoof) are representable from the start, because
retrofitting them would change every consumer's assumption that a contact has a subject. Rich deception — forged orders,
misinformation, inference about an opponent's beliefs — is later work (doc 10 §14 (c)).

**16B — one fused contact picture per side.** Worse: a fog-of-war leak by construction, which doc 09 Gate 3 exists to
catch. **16C — contacts are truth with a visibility flag.** Worse: a flag on truth cannot carry age, confidence or a
wrong answer, so no deception is ever possible.

### 17. Operation terminal settlement (Q5's other half)

**17A — one journaled resolution root fact, then idempotent consumer transitions (recommended).** An operation ends by
writing **one** `operation.resolved` root fact into the mechanical journal (T2). Every consequence is a separate,
**idempotent**, causally linked consumer transition keyed off that root: campaign outcome recording, front and control
mutation through `mush-faction` (`sim.rs:53-98`, `data.rs:465-486`), bond settlement, contract outcome selection
(`contract.rs:235-276`), task-force retirement. The operation stays **`Resolving`** until every required consumer
acknowledges, and only then reaches `Resolved`, so a crash mid-fan-out resumes rather than double-pays. A **truly atomic
mixed bundle** is adopted only when the kernel provides one: 08c §12's "K3 review" says in terms that a bundle carries
one state family's write set today — **no cross-crate atomicity to rely on**.

**17B — one transaction writes every consequence.** Worse: it claims an atomicity the tree does not have across three
crates. **17C — consequences applied by a staff step.** Worse: an operation that only settles when staff are awake,
again.

### 18. Content-pack lifecycle (Q6's other half)

**18A — no hot mutation of a published definition digest; versions coexist (recommended).** A definition is immutable
under its digest — the engine already treats `DomainRecord.driver` as an exact `DefinitionRef` including digest
(`mush-sim/src/data.rs:227-232`) and K3 validates exact descriptors at registration (`module.rs:622-634`). A change
publishes a **new version that coexists** with the old. Live units, blueprints, recipes and open contracts stay
**pinned** to the version they were created under until an **explicit migration or in-world refit action** moves them —
also T1's guarantee that a published hauling contract's route assumptions cannot shift underneath it.

**18B — edit packs in place and reload.** Worse: a digest shifting under a live contract breaks T1 and silently
re-prices open obligations. **18C — every reload migrates everything to the newest version.** Worse: balance changes
become retroactive and the audit trail a dispute needs is destroyed.

### 19. OOC proxy audit and participation storage (Q10's other half, R7/T13)

**19A — an audit-only server record maps driver token → account; journals keep only the token (recommended).** The
mechanical journal records the **in-world actor** and an **opaque decision-driver token** (T9's dual attribution) and
nothing else. A **server-side, audit-only** record links that token to the account; ordinary projections, renders and
exports cannot reverse it, and one staff verb can (R7). OOC participation history and matchmaking standing **consume the
audit record**, live outside the world, and **never become IC settlement**: T13's absent character earns no pay,
advancement, reputation, knowledge or property, and standing is a crew-call eligibility policy input only (R6).

**19B — the account id in the journal, filtered at render.** Forbidden by R3/R7: "never in a journaled, contract, or
canon record" is a storage rule, not a display rule. **19C — no audit record at all.** Worse: T11's dual attribution and
abuse investigation become impossible, and Q10's eligibility standing has no source.

### 20. Foreground-focus session semantics (T12/C21A)

**20A — focus is per account, atomic to acquire, with declared fallback on release (recommended).** **Multiple
connections may attach to one focus** — the session layer already fans out to many (`mush-session/src/lib.rs:5`) — so
"one focus" constrains *what is controlled*, never how many clients watch. Acquiring another focus **atomically
relinquishes the first** through the Q11 claim/release pair, so no interleaving can produce two. A **clean switch**
fires the released station's declared fallback **immediately** (T11); an **unclean disconnect** uses the existing
linkdead episode rather than a second model (`linkdead_grace`, 300 s, `mush-session/src/manager.rs:44-46`; doc 10 R5
makes the two rulesets share one fallback rule). **Accepting an invitation does not seize focus** — the grant is dormant
until activated, so a scheduled muster (T14) is accepted hours ahead without stranding the player.

**20B — one connection, one focus.** Worse: it breaks multi-connect, which the session layer already supports. **20C —
focus released lazily on timeout.** Worse: an NPC station with no driver for five minutes in a live tactical domain is
exactly what T11's "safe boundary" fallback exists to prevent.

### 21. Party subsystem ownership (T14, 08e answer 21)

**21A — one neutral party/muster subsystem, and it is E6a's `mush-party` (recommended).** 08f §2 ships `mush-party` in
**E6a** as a game-neutral crate depending on `{mush-core}` only, not a principal, never learning about money (`08f:90`,
`:94-95`). E7's musters, crew calls and roster snapshots use **that** crate: E7 either depends on it as shipped, or — if
E7 lands first — pulls that exact cut forward unchanged. **There is never a TacSim-specific party record.** (This
replaces Draft 0.1's "the party subsystem (E6b's, or E7d ships it)", which named the wrong cut and left ownership open;
§6.)

**21B — TacSim ships its own muster record.** Worse: two party subsystems means two invitation lifecycles, two expiry
rules and two snapshot semantics for one concept. **21C — musters are formations (Q9).** Worse: a formation is an
order-of-battle record with org authority; a muster is an ephemeral, consent-based grouping of *players*.

### 22. E7 combat scope (§11)

**22A — one coherent narrow vertical slice (recommended).** E7 proves the loop end to end with **one ground engagement
and one local-space interception**, both using **authored** units from the Q6 rules pack. Explicitly **later cuts**:
construction and manufacturing balance, the full weapon-family matrix, boarding actions, aerospace transitions between
ground and orbit, and fleet-scale tuning. §11's own framing: a proving loop that must first be balanced is not one.

**22B — full combined arms in E7.** Worse: doc 10 §8's scope is a multi-year target; E7 would never reach a playable
state to learn from. **22C — combat deferred entirely; E7 ships the economy half.** Worse: §11's proving question is
whether the *loop* is fun, and the loop's last arrow is "fight".

## 4. Proposed E7 cut structure

Eleven cuts, each independently CI-green and adversarially verified before the next builds on it (08a's rhythm); the E7
plan assigns them numbers. Feedback comes early because cut 1 is playable content, not scaffolding. Confirm or reorder.

| # | Crate(s) | One line | Verify axis | Depends on |
|---|---|---|---|---|
| **1** | `ruleset-tacsim` (skeleton) | the rules-pack loader and a **tiny corridor fixture** — three locations, two commodities, one recipe — so every later cut has a world to run in and the owner sees output first | pack digest identity, load determinism, fixture reproducibility | nothing |
| **2** | `mush-inventory` | pools and lots, locality bound to custody endpoints, split/merge, custody reservations (Q13, Q2) | lineage loss, quantity conservation, locality escape, reservation double-spend | E3 custody |
| **3** | `mush-industry` (deposits) | deposits, `ClaimRecord` + licence entitlements, surveying with bands, extraction and depletion (Q1, Q14) | band leakage, depletion arithmetic, claim contest and seizure | cut 2 |
| **4** | `mush-industry` (production) | facilities, recipes, capacity reservations shape-conformant to C8, work orders as scheduled journaled production (Q3) | restart mid-work-order, atomic consume, capacity double-book | cut 3 |
| **5** | `mush-sim` | actor-side **active-station focus**, control grants and leases, reach/traversal/safe-boundary handoff, republished links (item 10, Q20) | one-focus enforcement, stale-epoch intents, handoff at a boundary | the Q11 oracle |
| **6** | `mush-sim` | **carrier journeys** — the carrier root travels with interiors, occupants and cargo coherent — plus tactical **promotion/demotion** (item 9, Q15) | carrier topology across a leg, state preserved across promotion, active-domain limits | cuts 2, 5 |
| **7** | `mush-propagation` | detection and delivery over one propagation context, contacts with source/age/confidence, order envelopes and acknowledgement (items 4 + 14, Q4, Q16) | fog-of-war leaks, order supersession under ECM, ack round trip, false contacts | cut 6 |
| **8** | `mush-sim` + `mush-tactics` | the **minimal deterministic pulse driver**: `DomainClock::Pulse`, the domain tick, **one commit per domain per tick** within the K3 bundle limits (Q12) | determinism and replay, tick commit bounds, restart mid-tick | cut 7; the K3 lane |
| **9** | `mush-campaign` | declared operations as terms + runtime under one `OperationId`, formations as constraint sets bound to org scopes, bond as an economy hold, resolution fan-out (Q5, Q9, Q17) | freeze-at-activation, one-operation-per-unit, audience leaks, resolution idempotency | cut 8, `mush-faction`, E4b2 |
| **10** | `mush-campaign` + `mush-party` | crew complements and readiness obligations, **proxy seat offers/grants** with dual attribution and deterministic fallback, scheduled musters and open crew calls, participation facts (items 12, 15; Q10, Q19, Q21) | guardrail escapes, revocation at a safe boundary, absent-player settlement, audit irreversibility | cut 9; E6a's `mush-party`; the Q11 oracle |
| **11** | `ruleset-tacsim` + `mushd` config | the **recovery provider** (Q7) and the full Mars–Phobos–Ceres corridor seed pack: propulsion and route tables, ten commodities, recipes, combat formulas, insurance and warfare policy | the §11 proving loop end to end; pack versioning and digest stability under live contracts (Q18) | cuts 1–10; **E6c** for the market (below) |

**Dependencies to confirm.** (1) **E6c (`mush-market`, gated on the safe market projection) gates the corridor's "one
open market"** — E7 must not build a second one (08e answer 20). Recommended: the corridor ships with **direct vendor
sale** (E6b's vendor-from-stock path), enough to test the early physical loop — extract, haul, sell — from cut 3 onward;
the order book lands when E6c does. The "one open market" criterion is therefore **explicitly incomplete until E6c**,
and E7 builds **no substitute order book**. (2) **No E7 cut blocks on a K item.** Q3's interim is deliberately
K-independent and the sim-side staleness check is already the control epoch (`data.rs:635-640`); the only K dependency
is the **migration *review*** Q3 names, which gates nothing E7 ships. (3) **The shortest proving path** is cuts 1 → 2 →
3 → 4 → 6 → 11 (partial). If you would rather see a playable corridor sooner than a correct campaign layer, say so and
cuts 9/10 move behind a stub operation record.

## 5. What E7 hands back to E5/E6

- **Fact kinds `mush-sim` and `mush-tactics` register** on E4c's `FactProvider` (`facts.rs:62-77`; `modules.rs:148`):
  `sim.journey.arrived` / `.failed`, `sim.station.served` (R6/T13's participation fact, shape `{ actor, station, from,
  to }` per doc 10 §15), `sim.cargo.delivered`, `sim.docking.completed`, `tactics.sortie.started` / `.ended`,
  `tactics.unit.{disabled, abandoned, captured, destroyed}`, `tactics.salvage.recovered`, `campaign.operation.resolved`
  (Q17). Each owns a stable id space; the economy points *at* that journal and never duplicates it (T2).
- **`scene.outcome` stays E6b's** — composition-root data the server registers only when E6b ships it under facilitator
  authority (`contract.rs:311-322`). E7 consumes it and registers no scene outcome kind of its own.
- **The market projection stays E6c's** (listings, order books, quotes, the safe public projection — 08e answer 20; doc
  10 §14); E7 supplies **inventory locality** through Q13's layer and consumes quotes. **The party subsystem stays
  E6a's** (Q21): `mush-party`, `{core}` deps only, consumed by E7's musters.
- **Two small upstream asks**, both in doc 10 §15: the **scene IC time floor** in `mush-scenes` (doc 10 6A, now the Q8
  timeline's floor), and the **manual-cadence advance hook from the sim** (`advance_cadence_manual` exists; no caller).

## 6. External review (adjudicated)

An external review of Draft 0.1 was adjudicated by the lead; every point below was **accepted**, four with a sharpening,
and the folded result is this draft.

**Accepted as raised.** Ten decisions hidden inside the first twelve questions are now **Q13–Q22**. Four Draft 0.1
recommendations were demoted: **2A → 2D** (an account has no location), **5A → 5D** (terms and runtime version
separately), **8A → 8D** (the economy consumes time, it does not own it), **10A → 10D** (an OOC proxy is not an in-world
principal). 1B's rationale was corrected (T1 does not forbid stochastic extraction; 1A wins on legibility), 9A's
composition became a constraint set, Q3's migration became a review, Q21 named the party crate, and the five cuts became
eleven finer, independently verifiable ones.

**Sharpened by the lead.** (1) **2D extends the existing E3 custody model** from objects to quantities and lots — the
same `CustodyProvider` table and one-claiming-provider rule (`mush-economy/src/custody.rs:66`, `114-144`) — rather than
a parallel custody system. (2) **8D is kernel-owned, not campaign-owned**: the world timeline *is* doc 09 §2.5's
`ClockRef::WorldIc { generation }` and belongs to K4, the campaign layer setting only rate policy — what 08c §12 already
says `EconomicClock` is. (3) **10D shares E4b2's consent-hash helper rather than forking a second hashing discipline**.
(4) **12A's tick commit is bounded by the K3 bundle limits** (`authoritative.rs:28-38`), stated in the plan as the real
cap on a domain's active unit count.

**One doc 10 amendment proposed** — Q10's replacement for R6, quoted there in full, for the owner to accept into doc 10
Draft 0.4. Until they do, doc 10 R6 as written and this document's 10D disagree, and doc 10 governs.
