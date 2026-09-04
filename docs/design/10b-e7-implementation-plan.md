# 10b — E7 (TacSim) implementation plan

**Draft 0.1 — 2026-09-03.** The authoritative build plan for E7, in the shape of [08c](08c-e4-implementation-plan.md) and [08f](08f-e6-implementation-plan.md): scope, a cut table with verify axes, per-cut records / state machines / commands / tests, build discipline, decisions `D-E7-n`, constants, the owner's answers restated as binding, open questions with defaults, and a review-outcome placeholder.

**Binding inputs, not re-litigated here.** [10](10-tacsim-foundations.md) **Draft 0.4** — decisions **1A–7A** (§12), **C1A–C30A** (§18), addenda **T1–T16** (§13, §19), sharpenings **R1–R7** (§21, R6 as amended), the crate boundary (§14), the E4–E6 implications (§15), the backlog (§16), the environmental seam (§20). [10a](10a-tacsim-e7-owner-questions.md) **Draft 0.2, answered** — the owner accepted every recommended option: **1A 2D 3A 4A 5D 6A 7A 8D 9A 10D 11A 12A 13A 14A 15A 16A 17A 18A 19A 20A 21A 22A**, plus §4's eleven cuts and §5's handbacks. [08f](08f-e6-implementation-plan.md) (E6), [08a](08a-economy-build-status.md) + [08c](08c-e4-implementation-plan.md) (E4 as built), [08d](08d-e5-implementation-plan.md) §3.5–§3.6 (E5a as built), [09](09-rules-engine-contracts.md) (kernel contracts).

**One further input, and it is an input, not a plan.** The K branch carries `docs/design/08f-e7-implementation-plan.md`, a *requisition and production vertical slice*: work orders, facilities, reservations on the economy journal, typed-clock progress with an explicit generation-change policy, K5 offers, milestones E7a–E7e + E7k. This plan calls it **the production-slice proposal**, mines it for cuts 2–4, and adopts its §9 review checklist wholesale (§4). **Where it conflicts with a binding decision, the decision wins and the cut says so**; every override is marked **Ov** in §6.

**Two trees.** Citations marked *(main)* are `C:/Users/faeni/cc2/rustmush` at **committed HEAD `3967e12`**, not the working tree — an E5b build is in flight there and has moved lines in `mush-economy/src/{api,commands,contract_live,escalation,lib}.rs`, `mush-housing`, and `mush-server/src/modules.rs`; where that matters it is called out. Citations marked *(K)* are the scratch worktree `C:/Users/faeni/cc2/rustmush-k` at `1412f96` — the K branch head, **not yet merged**. E7 consumes K4a/K5a as landed there and must rebase onto them.

---

## 1. Scope — what E7 is, in the owner's terms

Doc 10 §1 is a **solar-system industrial warfare sandbox** where every battle has an economic cause. §11 names the first playable corridor — **Mars–Phobos–Ceres** — and its proving story: *extract water and metals near Ceres → hire players to haul to Phobos → fight or evade an interception → manufacture or refit a ground unit → load it aboard a carrier → deploy to Mars → complete a ground operation that changes control or access → salvage, repair, pay, repeat.* **22A** bounds E7 to **one coherent narrow vertical slice**: that loop end to end, with **one ground engagement and one local-space interception**, both fought with **authored** units from the rules pack. E7 builds:

- **A world made of packs, not code** (6A/18A) — immutable digest-referenced rules packs and once-applied campaign-seed packs; only what a table cannot express is Rust.
- **Quantity truth with a location** (2D/13A) — a neutral `mush-inventory` owning location-bound pools *and* lineage-bearing lots, every balance on an **E3 custody endpoint**, never a locationless account balance.
- **Industry that plans legibly** (1A/14A/3A) — deposits with deterministic curves and revealed *bands*; `ClaimRecord` tenure with licences as entitlements; facilities, recipes and work orders reserving matter, money and capacity through a **C8 shape-conformant** record.
- **One character, one active station** (C2A/C4A/C5A/20A) — billets stay many; the live control surface is the one focused station's K5a `ChannelGrant`, never a union.
- **Voyages as inhabited play** (C16A/C20A/15A) — the carrier root travels with its interiors, occupants, cargo and carried craft coherent; an interception **promotes** it into a live domain and demotes it at a quiet boundary with a durable summary.
- **One propagation layer** (4A/16A/R4) — detection and delivery over one physical context; contacts belong to a platform, never a side; orders are envelopes that must actually arrive.
- **A deterministic pulse** (2A/12A/T2) — `DomainClock::Pulse`, tactical ingress on the prepared K lane, **one commit per domain per tick**, and no economic transaction inside a pulse.
- **Declared operations that settle** (5D/T6/9A/17A) — frozen terms plus mutable runtime under one `OperationId`, formations as constraint sets bound to org scopes, a bond as an economy hold, one journaled resolution root fact with idempotent consumers.
- **Crew and seats** (C11A/C15A/T10–T16/10D) — named decision NPCs plus an aggregate complement, readiness on the operation cadence, **consent-grant** proxy seats with dual attribution and deterministic fallback.
- **A floor under loss** (3A/7A/T7) — a funded recovery organization leasing loaners against an eligibility test over *accessible* assets, never a faucet.

**Exit criterion.** The §11 proving loop runs end to end on the seed pack, with the ground engagement and the local-space interception both resolving through the pulse driver and settling through `campaign.operation.resolved`.

### 1.1 Explicitly out of E7

- **The belief engine.** Doc 10 §14 splits the knowledge dependency; E7 ships only (a), the deterministic contact/observer-projection layer. Rich NPC beliefs, inference, misinformation, forged orders and relationship-sensitive pricing are later (16A; doc 10 §14 (c)).
- **Full construction and manufacturing balance; the full weapon-family matrix; boarding actions; aerospace transitions between ground and orbit; fleet-scale tuning** (22A, verbatim). E7 authors *enough* recipes to run the loop — "a proving loop that must first be balanced is not one."
- **Kernel migration of reservations and settlement** — gated on three K items with no date (§2.4, §5). No cut 1–11 blocks on it (10a §4 dep 2).
- **The market.** `mush-market` — listings, order books, quotes, the safe public projection — is **E6c's** (10a §5; 08e answer 20; doc 10 §14). The corridor ships **direct vendor sale** (E6b's vendor-from-stock path) and E7 builds **no substitute order book**; §11's "one open market" is **explicitly incomplete until E6c**.
- **`scene.outcome`** — **E6b's** producer, registered by the server under adjudicator/facilitator authority (08f D-E6-7). E7 consumes it and registers no scene-outcome kind of its own.
- **The party subsystem** — **E6a's** `mush-party` (21A). There is never a TacSim-specific party record.
- **The proposal's E7e "bind every mutation to K5 legal offers"** — K5 proper has not landed (only K5a). 12A puts E7 on the prepared K lane now; the offer binding follows when K5 does, and no cut waits for it.

---

## 2. What E7 consumes as landed

### 2.1 K4a — the versioned world timeline *(K)*

- `WorldTimelineEpoch { real_anchor, ic_anchor, rate, generation }`, `rate` in IC seconds per real second, zero meaning a paused world clock — `crates/mush-core/src/clock.rs:167-176`. A new epoch anchors to the exact IC value the preceding generation produced at `real_anchor` (`:180-190`, `:333-341`), so **a rate change never reinterprets an earlier fact**.
- `ClockRef::{ Utc, WorldIc{generation}, SceneTurn{scene,generation}, DomainTick{owner,id,generation}, Registered{owner,id,generation} }` — `clock.rs:66-103`. **`DomainTick` is cut 8's clock.**
- `ClockStamp` with `checked_cmp` / `checked_duration_since`, which refuse across clock identities — `clock.rs:105-140`, errors `:141-157`.
- Append-only history with contiguity, monotonic-anchor and IC-continuity validation — `WorldTimeline::{from_epochs, current, stamp_at, next_rate_epoch, append}`, `clock.rs:193-341`; bounded at `MAX_WORLD_TIMELINE_EPOCHS = 4_096` (`:53`).
- Immutable per-generation documents plus an atomically updated head — `world_timeline_epoch_key` / `world_timeline_persistence_ops`, `clock.rs:429-465`; `WorldTimelineHead::validated_generation`, `:397-427`.
- Persistence and **boot hydration before modules** — `crates/mush-server/src/timeline.rs:19-78`: configuration seeds generation one only, persisted truth wins thereafter, a changed `[game]` clock warns rather than re-anchoring (`:57-65`).
- **Synchronous module-context read** — `Ctx::world_ic_now()`, `clock.rs:478-490`. Doc 09's record of the cut: `09:2137-2147` *(K)*.

### 2.2 K5a — station channel grants *(K)*, and the hazard it closes *(main)*

- `ChannelGrant { actor, subject, station, relation, owner, priority, channels, epoch }` — `crates/mush-core/src/world.rs:50-69`; `CommandSubjectLink` is now an alias (`:71-73`).
- **One grant per occupied station, no union and no max-priority promotion** — `crates/mush-sim/src/links.rs:18-59` *(K)* builds exactly one grant per seat on a placed, live unit, taking `channels` and `priority` from **that station's own** `StationSpec` and the unit's control generation as `epoch`; `refresh_actor_links` republishes from scratch (`:63-74`).
- **This is a real difference from main, and E7 depends on it.** `crates/mush-sim/src/links.rs:23-58` *(main)* still **unions channels across an actor's seats and takes the maximum priority** (`:39-54`) — exactly the promotion hazard doc 09 C8 names at `09:989-993` *(main)*. **E7 rebases onto K or cut 5 is unbuildable as specified.**
- **Exact station/channel/epoch revalidation for delayed work** — `World::channel_grant(actor, subject, owner, station)` and `channel_grant_is_current(.., epoch, required_channel)`, `world.rs:670-699` *(K)*: absence, revocation, an epoch bump or a narrowed channel set all fail closed. Doc 09's record: `09:2148-2153` *(K)* — note its own boundary: **per-channel active leases and safe-boundary transfer remain K6 work** (`09:2151-2153`, `:2210-2235`).

### 2.3 K3 audit closures E7 may assume *(K)*

- **K3.1a — checkpoint proof (closed):** memory and Redis atomically verify every exact replay post-image **before** deleting the full intent; a mismatch retains the intent and fails closed (`09:2056-2058`).
- **K3.1b — store-owned admission (closed):** the store refuses a second unresolved authoritative reservation, so two pending bundles cannot claim the same fresh object id (`09:2059-2061`).
- Bundle hard caps — `crates/mush-core/src/authoritative.rs:27-38` *(main)*: replay writes 1_024 (`:27-28`), definitions 256 (`:29-30`), witnesses 1_024 (`:31-32`), durable-outbox intents 256 (`:33-34`), payload 256 KiB (`:35-36`), bundle 16 MiB (`:37-38`). **These are the real cap on a domain's active unit count** (12A; 10a §6 sharpening 4), and cut 8 states the bound.

### 2.4 The three kernel-migration gates, and the K6 control gate

Before **any** production reservation or settlement moves off the economy journal onto authoritative K3 state families, all three must land (proposal §2.3; 08c §12's K3 review, `08c:1802-1815` *(main)*): (1) an authoritative bundle can carry the **mixed owner write set**, including the Ledger append; (2) ordinary module code can **synchronously read its typed state families outside an action**; (3) the **Ledger seal is folded into the same exact bundle** rather than acting as an external predecessor barrier.

Then a **migration review** opens. 3A adopts 08c §12 verbatim: it "is then still an explicit **owner decision**, not a mechanical port" (`08c:1810-1813`) — **the trigger opens a review, never an automatic migration**. §5 states what would move.

**Separately, K6 gates control handoff.** K5a grants "establish eligibility and stale-intent witnesses; they are not active exclusive leases" (proposal §2.4; `09:2151-2153` *(K)*). 10a **Q10/Q20** therefore let cut 10 ship seat **grants**, fallback and dual attribution while the **live handoff is stubbed behind the K6 gate**: E7 may assign title, maintenance and custody, but must not silently evict a live operator or transfer an active movement/weapons lease.

### 2.5 E4/E5a/E6-plan seams *(main; every line re-read at `3967e12`)*

| Seam | Where | E7's use |
|---|---|---|
| `CustodyProvider` — `handles` `:70`, `claims` `:73`, `move_into` `:85-91`, `move_out` `:93-100` | `crates/mush-economy/src/custody.rs:61-101` | cut 2 extends it from objects to quantities and lots (2D) |
| Exactly-one-claiming-provider (`resolve`, first `handles` wins then refuse if any other `claims`) and validate-then-mutate (`container_move`) | `custody.rs:103-131`, `:195-226` | unchanged; cut 2 adds no second custody model |
| `InteriorCustodyProvider`, the one registered endpoint | `crates/mush-sim/src/custody.rs:51`; `crates/mush-server/src/modules.rs:137-140` | the shipboard cargo endpoint, already moving with its ship |
| `Hold` = doc 09 C8's **money** specialization | 08c §3.2, §4.7 | the work-order budget and the operation bond; **never grows a resource field** (T4; doc 10 §15) |
| Outcome tables (`OutcomeTerms` `:227-256`, `ContractTerms` `:258-276`, `MAX_OUTCOMES` 8 `:58-62`); roles `:517-537`; `ConsentSurface` `:443-481`; roster lock `:757` | `crates/mush-economy/src/contract.rs` | claim contests, salvage claims, insurance, passage, recovery leases |
| **`consent_hash(issuer, consent) -> [u8;32]`** and `consent_intact` | `contract.rs:999-1030` (signature `:1014`), `:1063-1077` | **shared, not forked**, by cut 9's `OperationTerms` and cut 10's `SeatOffer`/`SeatGrant` (10D; 10a §6 sharpening 3) |
| `EconomicVisibility::Audience { members, announce_issuer }`, **`announce_issuer` default `false`**; `MAX_AUDIENCE` 32 | `crates/mush-economy/src/record.rs:339-365` (`:362-363`, `:322-323`) | T6's "notice of the window, not the plan" |
| `Encumbrance::{Lease `:528-536`, Disputed `:521-527`, Collateral `:537-542`}` | `record.rs:505-543` | recovery leases (7A), disputed title (T3), pledged claims (14A) |
| `InstrumentKind::{Currency, Commodity, Requisition, Entitlement}` `:50-61`; `scale` `:38-39` | `record.rs` | licences as `Entitlement` accounts (1A/14A); commodity value settlement |
| `ExternalRef { provider, kind, id, revision }` | `record.rs:296-316` | the economy points **at** the mechanical journal, never duplicates it (T2) |
| `CadenceClock` `:219-249` with `catch_up_cap` `:240-241`; `next_due` a **period index** `:222-225`; `CadenceSource` `:208-217`; `advance_cadence` `:264-295` | `crates/mush-economy/src/clock.rs` | readiness and upkeep obligations; the campaign cadence |
| `advance_cadence_manual` `:297-305` — **no caller at `3967e12`** outside its re-export (`lib.rs:152`) and one E4a test; the in-flight E5b build adds the staff verb `@econ/cadence advance`. **Either way there is no sim caller** | `clock.rs` | the hook doc 10 §15 owes: cut 6 (journeys) and cut 8 (sorties) become the first sim callers |
| `FactProvider::resolve(w, kind, id, revision)` `:76`, `FactTable` `:79-81`, unknown = unsatisfied `:35-40`; `with_facts` panics on a duplicate namespace (`module.rs:607-629`) | `crates/mush-economy/src/facts.rs:62-81` | **only `mush-jobs` registers today** (`modules.rs:148`), so every `sim.*`/`tactics.*` `Fact` condition resolves `None` — §10 |
| `create_system_obligation` (sig `:1125-1129`); `issue_title` (sig `:1338-1344`) | `crates/mush-economy/src/api.rs:1105-1203`, `:1330-1399` | subsidising the recovery provider (7A); work-order output, salvage, forfeiture |
| Producer-registered canon outcome kinds, source mandatory; wired **empty** today (`modules.rs:157`) | `contract.rs:296-322` | E7 registers none; `scene.outcome` stays E6b's |
| The graded escalation ladder (levels 0–3), every level-2 record carrying the recommended command with arguments filled in | 08c §4.6 | every E7 failure path; **stop only the subject** |
| `AuthorityGuard::Reservation { provider, id, revision }` `:1060-1064`; registry builder `:1417-1516`; per-`(owner, kind)` routing `:1534-1561` | `crates/mush-core/src/action.rs:1036-1071` | cuts 2 and 4 each register one validator under their own `OwnerId` |
| `ContactId` `:146-150`; `PerceivedRef::Contact { provider, id, generation }` `:655-669` — vocabulary with **no producer** | `action.rs` | cut 7 is the producer |
| The `brace` provider — `BraceProvider` `:470-537`, `impl ActionProvider` `:539-582`, typed state family `:592-621`, action + validator + provider registration `:623-635` | `crates/mush-tactics/src/module.rs` | the exact prepared-path shape cut 8 copies (12A) |
| `TacticalUnit` `:358-373`; sections `:314-326`; scalar cells `:220-245`; installed components `:328-352`; **separately titled major equipment** (`asset: Option<ObjId>`) `:340-351` | `crates/mush-tactics/src/unit.rs` | cut 4's output, cut 8's damage model |
| `Front` `:51-63`, `Clock` with `on_complete_control` `:65-98`; the region-control flip `lib.rs:235-274`; `ControlKind::{DeJure, DeFacto, Covert, Economic}` `data.rs:414-430`; `ControlRecord` `:461-483` | `crates/mush-faction/src` | cut 9's consequences; 14A's overlapping political authority |
| `CapabilityVocabulary` — an empty set accepts everything; the server's set is **not** empty and is a **typo catcher at the grant command**, not an authorization gate (`authorize` matches ids by equality) — `wiring.rs:78-93`; grant resolution `authorize.rs:143-200`; the server set `modules.rs:176-182` (economy + housing + `org.role.manage` + `org.grant.delegate`) | `crates/mush-org`, `crates/mush-server` | **`force.*` ids must be added to `org_capability_vocabulary()`** or granting one fails loudly at the command — cut 9. R2's "no **core** change" (10a Q5) still holds |
| `AccountRec { name, argon2_hash, created, characters, ssh_keys }` at `account:{lowername}` — **no immutable id, no reverse index** | `crates/mush-store/src/types.rs:718-741` | 11A's **two prerequisites**, cut 5 |
| E5a `mush-housing`: the `Tenancy` state machine, one pure `fn` per arrow, `Ended` reachable only through a `Resolution` | 08d §3.5 (D-E5-1) | **crew quarters reuse the tenancy shape**, not a new one |

**Absent repo-wide under `crates/*/src`** (re-checked): `Formation`, `TaskForce`, `TacticalOrder`, `mush-industry`, `mush-inventory`, `mush-market`, `mush-party`, `mush-propagation`, `ruleset-tacsim`; no deposit, recipe or lot type (the tokens appear only as `CustodyOp::Deposit` and in prose).

**Two gaps the survey surfaced, both E7 work.** (G1) **There is no `RouteDef` type at all** — a journey's route exists only as whitespace-separated `<room>[:secs]` tokens parsed inside `route_create` (`crates/mush-sim/src/commands.rs:1584-1648`), with `Leg { to, travel_s }` (`route.rs:45-52`) as the only shape. Cut 1's pack route tables therefore need a real route definition; cut 6 consumes it. (G2) **No `sim.*`/`tactics.*` fact provider exists**, so every contract `Fact` condition on a boundary fact answers `None` today — cuts 6, 8 and 9 register them (§10).

---

## 3. Cut structure

The eleven cuts of 10a §4, in order. Each is independently CI-green and adversarially verified before the next builds on it (§4). Cut 1 is playable content, not scaffolding, so feedback comes early. **The shortest proving path** is 1 → 2 → 3 → 4 → 6 → 11 (partial); if the owner wants a playable corridor sooner than a correct campaign layer, cuts 9/10 move behind a stub operation record (10a §4 dep 3).

| # | Crate(s) | One line | Verify axis | Depends on |
|---|---|---|---|---|
| **1** | `ruleset-tacsim` (skeleton) | the rules-pack loader and a tiny corridor fixture — three locations, two commodities, one recipe — so every later cut has a world to run in | pack digest identity, load determinism, fixture reproducibility | nothing |
| **2** | `mush-inventory` (new) | pools and lots, locality bound to E3 custody endpoints, split/merge/dissolve/crystallize, custody reservations | lineage loss, quantity conservation, locality escape, reservation double-spend | E3 custody |
| **3** | `mush-industry` (deposits) | deposits, `ClaimRecord` + licence entitlements, surveying with bands, extraction and depletion | band leakage, depletion arithmetic, claim contest and seizure | cut 2 |
| **4** | `mush-industry` (production) | facilities, recipes, capacity reservations shape-conformant to C8, work orders as scheduled journaled production | restart mid-work-order, atomic consume, capacity double-book | cut 3 |
| **5** | `mush-sim` | actor-side **active-station focus**, grants and leases, reach/traversal/safe-boundary handoff, republished links | one-focus enforcement, stale-epoch intents, handoff at a boundary | the Q11 oracle seam |
| **6** | `mush-sim` | **carrier journeys** — the root travels with interiors, occupants and cargo coherent — plus tactical **promotion/demotion** | carrier topology across a leg, state preserved across promotion, active-domain limits | cuts 2, 5 |
| **7** | `mush-propagation` (new) | detection and delivery over one propagation context; contacts with source/age/confidence; order envelopes and acknowledgement | fog-of-war leaks, order supersession under ECM, ack round trip, false contacts | cut 6 |
| **8** | `mush-sim` + `mush-tactics` | the **minimal deterministic pulse driver**: `DomainClock::Pulse`, the domain tick, one commit per domain per tick inside the K3 bundle limits | determinism and replay, tick commit bounds, restart mid-tick | cut 7; the K3 lane |
| **9** | `mush-campaign` (new) | declared operations as terms + runtime under one `OperationId`, formations as constraint sets bound to org scopes, bond as an economy hold, resolution fan-out | freeze-at-activation, one-operation-per-unit, audience leaks, resolution idempotency | cut 8, `mush-faction`, E4b2 |
| **10** | `mush-campaign` + `mush-party` | crew complements and readiness obligations, **proxy seat offers/grants** with dual attribution and deterministic fallback, musters and open crew calls, participation facts | guardrail escapes, revocation at a safe boundary, absent-player settlement, audit irreversibility | cut 9; E6a's `mush-party`; the Q11 oracle |
| **11** | `ruleset-tacsim` + `mushd` config | the **recovery provider** and the full Mars–Phobos–Ceres corridor seed pack; the timeline's rate policy | the §11 proving loop end to end; pack versioning and digest stability under live contracts | cuts 1–10; **E6c** for the market |

**Dependencies confirmed.** (1) **E6c gates the corridor's "one open market"**; direct vendor sale carries the physical loop from cut 3 onward (10a §4 dep 1). (2) **No E7 cut blocks on a K item** — 3A's interim is K-independent, the sim-side staleness check is already the control epoch (`mush-sim/src/data.rs:635-640`, main), and the only K dependency is the migration *review* (dep 2). E7 does, however, **rebase onto the K branch** for K4a/K5a (§2.2). (3) The proposal's milestones **E7a–E7e** are **superseded** by cuts 2–4 and this table (D-E7-25); its **E7k** becomes §5.

### 3.1 Cut 1 — `ruleset-tacsim` skeleton and the corridor fixture

**Records** (6A: everything expressible as a table is a versioned TOML content pack, in two kinds).

```rust
enum PackKind { RulesDefinition, CampaignSeed }
struct PackManifest { id: CanonicalId, version: u32, kind: PackKind, digest: DefinitionDigest,
                      requires: Vec<(CanonicalId, u32)> }
struct RulesPack { manifest: PackManifest, entries: BTreeMap<CanonicalId, PackEntry> }
struct RouteDef { id: CanonicalId, endpoints: (LocationId, LocationId), legs: Vec<LegDef>,   // ← G1
                  alignment_band: CadenceId, drive_class: CanonicalId, propellant: u64 }
struct SeedApplication { pack: CanonicalId, version: u32, digest: DefinitionDigest,
                         applied_at: ClockStamp, objects: u32 }          // `tacsim:seed:{pack}`
```

A **rules-definition pack** is immutable and referenced by digest: propulsion and route tables, the ten commodity families (4A), component definitions, recipes, formation kinds (9A), combat and insurance policy. A **campaign-seed pack** instantiates mutable world state **once**: corridor locations, organizations, deposits, facilities, inventories, NPCs. The distinction is load-bearing — *a rules pack is re-pinned and diffed, while re-running a seed pack would duplicate a mine* (6A). `[tacsim.*]` host config carries deployment switches and pack paths only, mirroring `[shadowrun.economy].catalog_paths` (08f §9).

**`RouteDef` closes G1.** There is no route type in the tree: a journey's route is only `<room>[:secs]` tokens parsed in `route_create` (`crates/mush-sim/src/commands.rs:1584-1648`, main) over `Leg { to, travel_s }` (`route.rs:45-52`). T1 needs a *definition* — route cost and duration must be deterministic functions of the route definition, drive and payload, declared departure, propellant plan, and the body-pair alignment band on a campaign cadence — so the definition is pack data from cut 1 and cut 6 consumes it.

**Invariants.** (i) A published digest is never mutated; a change publishes a **new version that coexists** (18A), and live units, blueprints, recipes and open contracts stay **pinned** until an explicit migration or in-world refit — T1's guarantee that a published hauling contract's route assumptions cannot shift underneath it. (ii) A seed pack applies at most once per `(pack, version)`; the `SeedApplication` document is the idempotency latch, written in the same batch as the objects. (iii) Loading is deterministic — same bytes, same digest, same iteration order. (iv) An unknown, oversized or digest-mismatched pack **fails closed at boot** and names the file. The engine already enforces the shape: `DomainRecord.driver` is an exact digest-bearing `DefinitionRef` (`crates/mush-sim/src/data.rs:227-232`, main) and K3 validates exact descriptors at registration (`crates/mush-core/src/module.rs:622-634`, main).

**The tiny corridor fixture.** Three locations — a Ceres extraction site, a transfer node, a Phobos yard; two commodities — `metals` and `structural_alloys`, both from 4A's ten; one recipe — `alloy_smelt@1`; one `RouteDef` joining Ceres to Phobos. Enough for cuts 2–4 to run and for the owner to see output first.

**Commands.** `@tacsim/pack list|show <id>|verify` · `@tacsim/seed <pack> [dry|commit]` (staff) · `@tacsim/pin <subject>=<pack>@<version>` (staff, for a migration or refit).

**Tests (~22).** *Digest:* identical bytes → identical digest; a whitespace-only edit changes it; a reordered table does not (canonical encoding); `MAX_PACK_ENTRIES` refuses. *Load:* two loads are byte-identical in memory; an unknown key, a missing `requires` dependency and a `requires` version mismatch each fail closed naming the file. *Seed:* `dry` writes nothing and lists every planned object; `commit` applies once; a second `commit` is a HIT writing nothing; a *different version* of the same pack is refused with the applied version named; a crash between object writes and the latch converges on reboot. *Pinning:* a subject pinned to `v1` is unaffected by publishing `v2`; both versions load concurrently; an unpinned consumer of a retired version fails closed rather than silently upgrading. *Routes:* a `RouteDef`'s duration is a pure function of definition, drive and payload; two evaluations agree; an unknown drive class refuses. *Fixture:* the three locations, two commodities, recipe and route exist after `commit` and nowhere else; the fixture is reproducible from an empty store; boot with a corrupt pack refuses to start.

**Verify lenses.** **P (pack identity)** — can two byte streams produce one digest, or one stream two? **S (seed idempotency)** — can any interleaving or crash duplicate a seeded object or lose the latch? **B (boot fail-closed)** — does any malformed pack degrade silently instead of refusing?

**Hands to cut 2:** the commodity family list, `alloy_smelt@1`, the `RouteDef` type, and the digest discipline every later definition reference uses.

### 3.2 Cut 2 — `mush-inventory`, quantity truth with a location

**Ov, stated.** The proposal §4 gives `WorkOrder` a `material_source: Option<AssetContainerRef>` and speaks of a "material vault". **13A/2D override this**: quantities are neither asset containers nor account balances. A neutral `mush-inventory` is *the single source of quantity truth*, bound to **custody endpoints**, and industry, markets, contracts and simulation consume it — **none keeps a private stock number**. The proposal's material-vault quantities live here; `material_source` becomes an `EndpointRef`.

**Records** (2D fixes the fields exactly: **not** `legal`, **not** `custody`).

```rust
struct EndpointRef { provider: OwnerId, object: ObjId }     // an E3 custody endpoint
struct Pool  { endpoint: EndpointRef, commodity: CanonicalId, quantity: u64, revision: u64 }
struct Lot   { id: LotId, commodity: CanonicalId, quantity: u64, grade: Grade, lineage: Lineage }
enum  Lineage { Origin { source: ExternalRef }, Split { parent: LotId },
                Merge { parents: Vec<LotId> },              // ≤ MAX_LOT_PARENTS
                Tranche { tranche: TrancheId } }
struct PoolTranche { id: TrancheId, endpoint: EndpointRef, commodity: CanonicalId,
                     absorbed: Vec<LotId>, quantity: u64 }  // immutable lineage root
struct InventoryReservation {                               // doc 09 C8, exact fields (09:997-1008)
    id: ReservationId, owner: OwnerId, resource: ResourceRef, holder: ScopedIdempotencyKey,
    amount: u64, revision: u64, expires: ClockStamp,
    state: ReservationState,                                // Held | Consumed(commit) | Released | Expired
}
```

**Custody is the endpoint's, not the lot's** (2D): a lot has one location and one custodian because its endpoint does. **Stolen or disputed status follows the *title* record** (`Encumbrance::Disputed`, `record.rs:521-527`, main; E6b's `report_stolen`). **Embargo legality is a jurisdiction/policy test at transaction time**, not a field travelling with matter — the same cargo is legal one hop away. **Licensing is the acting principal's entitlement**, not the cargo's (1A/14A).

**Verbs.** `credit`/`debit` a pool · `split(lot, n)` · `merge(lots)` · `transfer(what, to_endpoint)` · `reserve`/`consume(commit)`/`release`/`expire`. Interconversion is **one journaled, attributed act each way** (2D/T5): `dissolve(lot)` creates or updates an immutable `PoolTranche` and credits the pool; `crystallize(pool, n)` mints a lot whose **parent is that tranche**. **Lineage is never discarded — only coarsened in the active projection.**

**Invariants.** (1) Conservation under checked arithmetic; a `debit` below available (balance minus live holds) refuses. (2) Locality: no verb moves quantity except *between endpoints*; there is no endpoint-free balance. (3) The custody rules are E3's, unchanged — exactly one claiming provider (`custody.rs:103-131`), validate-then-mutate (`:195-226`); a hold that moves with a ship needs no second mechanism. (4) Lineage is append-only and acyclic, bounded by `MAX_LOT_PARENTS`/`MAX_LINEAGE_DEPTH`. (5) The reservation state machine is doc 09 C8's exactly; availability subtracts every live hold; the consuming action verifies holder **and** revision and applies the post-image in the same batch. (6) `mush-inventory` registers an `AuthorityGuardKind::Reservation` validator under its **own** `OwnerId` (`action.rs:1417-1516`, `:1534-1561`, main) — the `brace` registration pattern (`mush-tactics/src/module.rs:592-635`, main).

**Commands.** `@inv/show <endpoint>` · `@inv/lots <endpoint> [commodity]` · `@inv/lot <id>` (lineage, paged) · `@inv/move <what>=<endpoint>` · `@inv/split <lot>=<n>` · `@inv/merge <lot>,<lot>…` · `@inv/dissolve <lot>` · `@inv/crystallize <endpoint>/<commodity>=<n>` · `@inv/reservations <endpoint>`.

**Tests (~28).** *Conservation:* every verb sums; a `debit` past available refuses and writes nothing; `u64` overflow on merge refuses; a zero-quantity split refuses. *Lineage:* split → merge → split reconstructs both parents; `dissolve` then `crystallize` leaves the tranche as the child's parent and the original lot id reachable; a merge over `MAX_LOT_PARENTS` refuses; a cycle is unconstructible; coarsening the projection deletes no stored parent; `MAX_LINEAGE_DEPTH` refuses. *Locality:* no verb produces an endpoint-free pool; moving a ship moves its hold's contents by construction; a transfer to a destroyed endpoint refuses; a transfer between endpoints claimed by two providers refuses. *Reservations:* two orders cannot hold one unit; `consume` twice with the same commit is a HIT; a stale revision refuses; `release` after `consume` refuses; expiry is idempotent and survives restart; availability is balance minus live holds at every read. *Restart:* every record re-enters through its constructor on load (E5a lens-A: deserialization must not walk past the caps); a corrupt lot document is skipped **loudly** and its endpoint marked, never silently zeroed. *Custody:* a lot in a vault inherits the vault's access; `may_take` refuses a non-custodian.

**Verify lenses.** **C (conservation and lineage)** — can any sequence create, destroy or orphan quantity, or lose a parent? **L (locality)** — can a balance exist without a live endpoint, or move without a custody act? **R (reservation)** — can two holders spend one unit, or a `Consumed` hold be released?

**Hands to cut 3:** the endpoint-bound pool/lot API extraction writes into, and the C8-conformant reservation shape cut 4 reuses for capacity.

### 3.3 Cut 3 — `mush-industry`: deposits, claims, survey, extraction

**Records** (1A and 14A, verbatim field lists).

```rust
struct Deposit { id: DepositId, region: RegionId, commodity: CanonicalId, grade: Grade,
                 accessibility: Accessibility, reserve: u64, curve: YieldCurve,
                 surveyed_bands: BTreeMap<PrincipalId, SurveyBands> }
struct SurveyBands { grade_class: BandId, reserve_class: BandId, at: ClockStamp, fact: ExternalRef }
struct ClaimRecord { id: ClaimId, deposit_or_area: ClaimSubject, issuer: PrincipalId,
                     holder: PrincipalId, exclusivity: Exclusivity, priority: u32, term: ClockStamp,
                     transferability: Transferability, contest_state: ContestState, revision: u64 }
enum ContestState { Uncontested, Contested { contract: ContractId }, Suspended { by: PrincipalId },
                    Seized { by: PrincipalId, at: ClockStamp } }
```

A **licence is not tenure**: it is an `InstrumentKind::Entitlement` account issued by a government principal (`record.rs:50-61`, main) with campaign-cadence expiry, proving **authorization** to survey, extract or export — checked **at transaction time** against the *acting principal* (1A/14A/2D).

**Yield is deterministic** (1A). `curve` maps cumulative extraction to per-cycle yield; depletion is arithmetic, not a roll. **Variation comes from discrete campaign events** — accessibility change, equipment condition, collapse, licence suspension — each an attributed journaled act moving effective yield, never a per-period roll. A **survey reveals bands, never the exact reserve**, and the revealed band is per principal and is itself a recorded fact.

**Extraction** is scheduled, journaled **lot creation** on a deterministic key — doc 09 C8 already lists production and upkeep among bounded-idempotency-key actions (`09:1019-1028`, main). One cycle produces one `Lot` with `Lineage::Origin` at the extractor's endpoint (cut 2) and debits `reserve` in the same batch.

**Tenure lifecycle** (14A). A claim is **transferable**, **pledgeable** as collateral (`Encumbrance::Collateral`, `record.rs:537-542`, main), **contestable** through an E4 contract with a `Before` deadline and a named adjudicator (T3's shape), **suspendable** and **seizable** by the issuing authority. Overlapping political authority needs no fiction that an entitlement balance is land title: two governments may each issue a claim over one area, and `ControlKind::{DeJure, DeFacto}` (`mush-faction/src/data.rs:414-430`, main) says which is worth anything on the ground.

**Owner abandonment lands here** (T3; doc 10 §15; 08f §1.1 keeps it "in E7 with the mechanical recipe written out"): a unilateral `AssetEffect` setting `title_owner` to none with custody unchanged, a **direct attributed asset action** that need not masquerade as a bilateral contract — next to the claim seizure that needs the same unilateral title effect. The salvage claim that turns possession into title stays an ordinary contract with a `Before` contest deadline and a named adjudicator.

**Commands.** `@deposit/list <region>` · `@deposit/show <id>` (bands only, per viewer) · `@survey <deposit>` · `@claim/file <subject>=<issuer>` · `@claim/show|transfer|pledge|release` · `@claim/contest <claim>` · `@claim/suspend|seize <claim>` (issuer) · `@extract/start <deposit>=<facility>` · `@asset/abandon <object>` · `@licence/show <principal>`.

**Tests (~26).** *Band leakage:* an unsurveyed principal sees no band; a surveyed one sees **only** their band and never `reserve`, `curve` or another principal's band; a render at every verbosity is asserted not to contain the exact reserve; staff is the only exact read; a transferred claim does not transfer the transferor's bands. *Depletion:* cumulative extraction matches the curve over 200 cycles; a depleted deposit yields zero and refuses a new cycle; concurrent extractors cannot over-draw (each cycle is one reserve-consume); an accessibility event changes the *next* cycle, never a completed one; overflow refuses. *Claims:* two claims over one deposit coexist with distinct issuers and priorities; an exclusive claim refuses a second from the same issuer; a contest opens exactly one contract and moves `contest_state`; a contest settling for the challenger transfers the claim once, idempotently; suspension blocks extraction while leaving the record; seizure is attributed; a pledged claim cannot be transferred while the encumbrance stands; a claim past `term` stops authorizing and does not vanish. *Licences:* extraction without the entitlement refuses; an expired licence refuses at transaction time though the claim stands; an org-held licence authorizes a member acting on the org's scope, not the member personally. *Abandonment:* title goes to none, custody is unchanged, the act is attributed, and a re-run is a HIT. *Restart:* an extraction cycle interrupted mid-batch produces either no lot or exactly one.

**Verify lenses.** **F (fog)** — can a viewer learn a reserve, a curve or another principal's band through any render, error, or arithmetic difference? **D (depletion)** — can total extracted exceed the reserve under concurrency, restart or event ordering? **T (tenure)** — can a claim be transferred, extracted under, or contested by someone the authority chain does not permit?

**Hands to cut 4:** lots at a facility endpoint, and the licence check the work order re-runs.

### 3.4 Cut 4 — `mush-industry`: facilities, recipes, work orders

This cut **is** the production-slice proposal, folded onto the binding decisions. Its §4 shapes, §5 clock semantics, §7 scenario and §8 E7a–E7d gates are adopted; four things change.

**Ov.** (1) **Materials come from `mush-inventory`, not an asset container or "material vault"** (2D/13A) — `material_source: EndpointRef`. (2) **The value reservation is an E4 `Hold`, not an industry reservation**: T4 and doc 10 §15 keep holds money-only and forbid a resource field, while the proposal lists money, materials and capacity as undifferentiated "reservations". (3) **The capacity reservation is 3A's C8 shape-conformant record in `mush-industry`, holder = the work order's deterministic key**, with a named migration **trigger that opens a review, never an automatic migration** (§5). (4) **The campaign — not the ruleset — registers a restamp policy** (8D); the proposal's §3 boundary gives the ruleset "clock choice", which 8D reserves to the kernel (generations) and the campaign layer (rate policy), leaving the ruleset a pack value.

**Records** (proposal §4, amended).

```rust
struct RecipeRef  { owner: OwnerId, id: CanonicalId, version: u32, digest: DefinitionDigest }
struct FacilityRecord { object: ObjId, owner: PrincipalId, capabilities: BTreeSet<CanonicalId>,
                        capacity_revision: u64, policy_revision: u64,
                        active_slots: u32, maximum_slots: u32 }
struct WorkOrder { id: WorkOrderId, revision: u64, requester: PrincipalId, beneficiary: PrincipalId,
                   facility: ObjId, recipe: RecipeRef, quantity: u32,
                   budget_account: AccountId,             // money → an E4 Hold (T4)
                   material_source: Option<EndpointRef>,  // ← was AssetContainerRef (2D)
                   destination: DeliveryTarget, holds: Vec<HoldId>,
                   reservations: Vec<ReservationId>, schedule: Option<ProductionSchedule>,
                   state: WorkOrderState, visibility: EconomicVisibility }
struct ProductionSchedule { started: ClockStamp, due: ClockStamp, clock_policy: CanonicalId,
                            progress_units: u64, total_units: u64 }
enum WorkOrderState { Draft, AwaitingApproval { request: ApprovalRequestId }, Ready, Running,
                      Interrupted { cause: InterruptionCause }, AwaitingDelivery,
                      Completed { commit: CommitToken }, Cancelled, Failed }
```

`active_slots` is an **advisory counter, not authority** — the proposal's own checklist says so. Availability is derived from live capacity reservations at every read, exactly as `held_amount` is a re-derived projection in E4 (08c D-E4-1).

**What an accepted order holds** (proposal §4, re-typed): money/requisition value as an E4 `Hold`; each conserved material input as a cut-2 inventory reservation; one facility slot or measured capacity as an industry reservation; any unique source asset being repaired or refit as an E4 custody lock/`Encumbrance`; an optional bounded destination capacity claim as an inventory reservation on the destination endpoint. **Every one uses the work order's deterministic holder key.** Completion consumes all required holds in the same transition that makes output available; cancellation and rejection release idempotently. A failed delivery may keep a completed output in `AwaitingDelivery` but **may not re-run production or consume the inputs twice**.

**"Consume atomically" before K3's mixed write sets** (3A): one plan, one batch, one journal-authoritative work-order document naming every reservation it needs, a single companion-carrying terminal batch, **no fallible step after the first install** — E4b2's plan-first rule, the strongest atomicity the tree offers (08c §5.1).

**Clock semantics** (proposal §5, adopted; 8D's ownership). `started` and `due` are captured on one exact generation; scheduler comparisons are legal only while the provider returns that identity (`ClockStamp::checked_cmp`, K `clock.rs:119`). On a generation mismatch the production provider performs **one explicit campaign-selected action**: restamp preserving remaining IC duration; restamp preserving the previously computed real due instant; or **pause as `Interrupted { TimelineChanged }`**. **The first slice uses the third, safest policy unless the campaign module registers one of the first two** (D-E7-8). A restamp is a durable attributed transition; **merely observing a new generation never edits the order in place**.

**Commands.** `@facility/list|show <id>` · `@wo/draft <recipe>=<facility>` · `@wo/set <id>/<field>=<value>` · `@wo/quote <id>` (deterministic price, duration, requirements, opaque offer) · `@wo/submit <id>` · `@wo/cancel <id>` · `@wo/show <id>` · `@wo/list [facility|mine]` · `@wo/deliver <id>` · `@wo/restamp <id>=<policy>` (campaign authority).

**The first vertical scenario** (proposal §7, re-based on cut 1's corridor fixture rather than a free-floating motor pool). A division member with requisition permission drafts one `alloy_smelt@1` at the Phobos yard, selecting the division wallet, an inventory endpoint holding `metals`, and a hangar destination; the server returns deterministic price/duration/requirements and an opaque legal offer; submission revalidates membership, role scope, account and endpoint authority, recipe digest, facility revision, destination capacity and offer binding; below-threshold requests reserve and start, above-threshold ones enter the existing org approval and reserve only after it; a duplicate submit joins the same order and a competing order cannot spend the held funds or material or overbook the second slot; restart during `Running` hydrates the same holds, schedule generation, progress and timer key; losing facility access interrupts rather than completing remotely; a timeline generation change pauses or restamps per registered policy; completion consumes holds once, constructs the output, assigns title and maintenance liability, places it at the destination and emits one causal completion event; **title alone grants no station and no control** (proposal §6).

**Tests (~30).** *Authorization:* a non-member, a member without the scope, a suspended scope, a frozen account and a closed endpoint each refuse at submit and write nothing; approval above the threshold reserves only after approval; approval replay is a HIT. *Reservation:* two orders contend for the last slot and exactly one starts; a duplicate submit joins the same order; a competing order cannot spend held funds or material; cancellation releases every hold exactly once and a second cancel is a HIT; expiry releases idempotently. *Atomicity:* every injected cut point in the terminal transition either retains the pre-state or converges to exactly one output; the allocator cannot reuse the output object id across restart (K3.1b); a probe re-entering asserts no `?` remains after the first irreversible mutation. *Clock:* a duplicate timer fire is a HIT; an out-of-order fire converges; a large jump is bounded by catch-up; **a generation change with no registered policy pauses as `Interrupted { TimelineChanged }` and never silently moves `due`**; a registered restamp preserves remaining IC duration as one attributed transition; observing a new generation without a transition edits nothing; comparing stamps across generations refuses. *Drift:* a recipe digest change makes a draft stale; a `capacity_revision` bump makes a `Ready` order revalidate; a `policy_revision` bump re-runs the licence check (cut 3). *Delivery:* a full destination leaves `AwaitingDelivery`, and a retry delivers without re-running production or re-consuming inputs. *Leakage:* a projected work order carries no hidden org, division, account or endpoint facts; the offer id is opaque; a malformed or oversized record fails closed. *Restart:* hydration at every state.

**Verify lenses.** **A (atomicity and contention)** — can any interleaving, retry or crash overspend, overbook, or produce two outputs? **G (generation)** — can a rate change silently move a deadline, teleport progress, or compare incomparable stamps? **P (projection)** — does any render, error or offer expose a fact the viewer may not know?

**Hands to cut 5:** a built unit with title, maintenance and custody assigned and **no station** — the exact seam that makes cut 5's focus model the only thing granting control.

### 3.5 Cut 5 — `mush-sim`: the active-station focus

**The correction** (doc 10 §17 item 1). Station assignments are host-keyed (`HostAssignments` over `StationAssignment`, `crates/mush-sim/src/data.rs:455-462`, `:446-453`, main) and the derived index is actor → *set* of units (`:502-503`). K5a stopped the channel-union hazard by projecting one grant per occupied station (§2.2). What is still missing is an **actor-side active focus** so only one station supplies live human control. Preserve the existing records as **billets**; add the focus above them.

```rust
struct ActiveFocus { actor: ObjId, unit: ObjId, station: StationId,
                     acquired_at: ClockStamp, epoch: u64, reach: ReachProof }
enum ReachProof { Console { room: ObjId },                            // C4A: a physical console…
                  RemoteLink { terminal: ObjId, path: EnvelopeId } }  // …or a valid remote link
enum FocusRelease { CleanSwitch, Traversal, SafeBoundary, Revoked, Linkdead }
```

**Invariants.** (1) **At most one `ActiveFocus` per actor, and one foreground focus per account** (C21A/T12/20A) — acquiring another **atomically relinquishes the first** through the account provider's claim/release pair, so no interleaving produces two. (2) **Multiple connections may attach to one focus**; the session layer already fans out to many (`mush-session/src/lib.rs:5`, main), so "one focus" constrains *what is controlled*, never how many clients watch (20A). (3) A billet without focus is **eligibility only**: it still projects its `ChannelGrant` (so a delayed intent can be revalidated and refused), but the command path requires the grant's station to be the focused one. (4) Station changes **relinquish the former lease, require physical traversal when consoles occupy different places, and complete at a safe pulse boundary** (C5A; doc 10 §18.1); NPC or automation fallback maintains the old station through the handoff. (5) **Ownership alone never satisfies reach or control.** (6) A **clean switch fires the released station's declared fallback immediately**; an **unclean disconnect** uses the existing linkdead episode rather than a second model (`linkdead_grace`, 300 s, `mush-session/src/lib.rs:57-72`, `manager.rs:44-46`, main), and doc 10 R5 makes the two rulesets share one fallback rule. (7) Revocation or any control-surface change **bumps the unit's control epoch** (`mush-sim/src/data.rs:615-622`, main), so stale intents fail `channel_grant_is_current` (K `world.rs:684-699`). (8) **Accepting an invitation does not seize focus** — a grant is dormant until activated, so a scheduled muster is accepted hours ahead without stranding the player (20A; cut 10 depends on this).

**The account seam, pulled forward here** (11A). `mush-core` defines a synchronous provider shaped like `FactProvider`/`CustodyProvider` (`mush-economy/src/facts.rs:62-81`, main), implemented by the server/store layer: `same_account(a, b) -> Option<bool>`, `all_accounts_distinct(characters)`, and **atomic claim and release of the account's foreground focus**, with synchronous updates when characters are created, linked or removed and when accounts are merged or renamed. **`None` = unknown and every calling policy fails closed.** Core, not `mush-org`, because roster lock, side assignment, parties and seat grants should not each gain an org dependency, and an account has no org (11A). **Cut 5 ships the seam and the focus half; cut 10 adds the distinctness consumers** (§9 q6). Its **two prerequisites** land here: **(i)** an immutable opaque account id on `AccountRec`, stable across a rename (`mush-store/src/types.rs:718-741`, main, has none — the lowercased name *is* the key, so a rename is an identity change today), and **(ii)** a character → account-id map hydrated at boot so the oracle answers synchronously inside a world command. **Neither puts the id in a journaled or canon record** (R3/R7).

**Commands.** `+focus <unit>/<station>` · `+focus/release` · `+focus` · `+stations` (my billets, with reach and focusability) · `@station/fallback <unit>/<station>=<who>` · `@account/audit <a>=<b>` (staff — answers "same account?" **without printing an identity**, R7).

**Tests (~26).** *One focus:* two `+focus` in one tick leave exactly one; a second connection attaches to the same focus; two connections issuing conflicting orders both resolve against one focus; an oracle answering `None` refuses the acquire. *Stale epoch:* an intent queued under grant epoch `e` fails after a revocation, a channel narrowing, an unplacement and a station removal; a grant on a *different* station of the same unit does not satisfy the check; priority is never taken from a different grant than the one supplying the channel (K5a). *Reach:* a console in another room refuses without traversal; a remote link refuses when the delivery path is down (cut 7 tightens this; cut 5 accepts a stub predicate); ownership without a billet refuses. *Handoff:* a switch mid-pulse completes at the boundary, not before; the released station's fallback is live before the boundary passes; a linkdead actor keeps the seat for `linkdead_grace` then falls back; a clean switch does not wait for the grace. *Billets:* an actor with three billets projects three grants and controls one; a non-focused billet's command refuses **naming the focused station**. *Restart:* focus is re-derived, not resurrected — after a restart no actor holds a focus until they re-acquire, and the declared fallback drives the seat. *Account:* a rename preserves the immutable id; the boot map answers inside a world command; the id appears in no journal (a byte scan asserts it).

**Verify lenses.** **F (one focus)** — can any interleaving, reconnect, crash or proxy path produce two live control surfaces for one account? **E (epoch)** — can a stale intent execute after any control-surface change? **H (handoff)** — can a seat be empty, doubly-held, or held past a revocation across a boundary?

**Hands to cut 6:** the focus and fallback semantics a carrier's crew keeps across a journey leg and across promotion into a live domain.

### 3.6 Cut 6 — `mush-sim`: carrier journeys, promotion and demotion

**The correction** (doc 10 §17 item 2, §16 item 9, C16A). A `Journey { traveler, legs, cursor, started_at, status, paused_at, grace_until, destination, by }` (`crates/mush-sim/src/route.rs:67-90`, main) **refuses any traveler that is placed, quarantined, a domain host, an interior space, a mount host or a station host** (`commands.rs:1529-1564`; the shared predicate `is_sim_bound`, `data.rs:678-690`; boot eligibility `boot.rs:728-763`). Replace that with a **carrier-root invariant**: the root is **not directly placed in a live domain while traveling**, but its station infrastructure, anchored interiors, occupants, mounts, cargo and carried craft **remain coherent and move with it**. The carrier root owns **one** journey; nothing aboard gets a duplicate record (C16A; doc 10 §18.4).

```rust
struct CarrierJourney { journey: JourneyId, root: ObjId, route: RouteDef, legs: Vec<Leg>,
                        departed: ClockStamp, due: ClockStamp, propellant: ReservationId,
                        catch_up: CatchUpPolicy, state: JourneyState }
enum JourneyState { Cruising { leg: u8 }, Promoted { domain: DomainId, resume: ResumePlan },
                    Arrived, Diverted { to: CanonicalId }, Captured { by: PrincipalId }, Failed }
struct DomainActivation { domain: DomainId, trigger: PromotionTrigger, at: ClockStamp,
                          clock: ClockRef /* DomainTick */, units: u32 }
enum PromotionTrigger { PlayerPresent, CredibleContact, Interception, OperationActivated }
struct DomainSummary { domain: DomainId, from: ClockStamp, to: ClockStamp, contacts: u32,
                       damage: Vec<UnitDamageLine>, facts: Vec<ExternalRef> }
```

**Journey semantics.** T1: cost and duration are deterministic functions of the `RouteDef` (cut 1), drive and payload, declared departure, propellant plan and the body-pair **alignment band on a campaign cadence** — no wall-clock randomness changes a contract after publication. Survey **6A**: a scene cannot hold or delay a journey. At arrival, in order: (1) the journey commits at its scheduled campaign time; (2) the carrier changes operational location; (3) active onboard scenes receive a **typed arrival event**; (4) their local time advances to **at least** the arrival timestamp; (5) the scene continues as a new chapter, keeping participants and transcript. This needs the **scene IC time floor** — `scene_ic_time` is `ic_started_at + (round − 1) × round_seconds` with **no floor or clamp** (`crates/mush-scenes/src/data.rs:171-178`, main) — see §10.

**Promotion and demotion** (15A). *The governing rule: the system never runs tactically as a whole.* A domain promotes to `DomainClock::Pulse` on **four deterministic triggers** — a player character present in a contested context, a **credible contact** between opposed parties, an **interception** along a journey leg, or the **activation of a declared operation** (cut 9). It demotes **only after a quiet boundary** (no contacts, no pending orders, no unresolved damage for `QUIET_BOUNDARY`) and once a **durable summary** is written. **Hard limit `MAX_ACTIVE_DOMAINS`** with a stated policy at the limit (§9 q1). **Orders, contacts, damage and position are preserved across promotion and demotion in both directions — demotion changes cadence, never state.** `DomainClock::Passive` is the only variant today (`mush-sim/src/data.rs:210-216`, main), and `DomainRecord.driver` is an exact digest-bearing `DefinitionRef` (`:218-241`, `:227-232`), so `Pulse` arrives as a second variant plus a driver definition, not a rewrite.

**The manual-cadence hook** (doc 10 §15). `advance_cadence_manual` exists (`mush-economy/src/clock.rs:297-305`, main) with **no caller at `3967e12`** beyond its re-export and one E4a test; the in-flight E5b build adds a staff verb, and **neither is a sim caller**. This cut is the first: a journey boundary advances the T1 alignment-band cadence; cut 8 adds the sortie boundary for doc 10 §9's per-sortie maintenance.

**Commands.** `@journey/plan <carrier>=<route>` · `@journey/depart <carrier>` · `@journey/show` · `@journey/divert <carrier>=<route>` · `@domain/list` · `@domain/promote <where>` · `@domain/summary <domain>`.

**Tests (~28).** *Topology:* a carrier with three anchored interiors, six occupants, two mounted craft and a loaded hold departs and arrives with every relationship intact; an occupant who walks between compartments mid-leg is in the right place at arrival; a craft launched mid-leg is refused while cruising and permitted once promoted; destroying an anchored interior mid-leg orphans no occupant; the root is never directly placed in a live domain while `Cruising`. *Arrival:* a scene open aboard receives the typed arrival event and its IC time floor rises to the arrival stamp; **a scene cannot delay arrival** (6A) — a 40-round scene and a due arrival both resolve, arrival first; a journey arriving during a promotion completes at the safe boundary; the alignment cadence advances exactly once per boundary. *Promotion:* each of the four triggers promotes exactly once; a fifth trigger on an active domain does not re-promote; at `MAX_ACTIVE_DOMAINS` the next trigger queues and re-evaluates rather than dropping; demotion refuses while a contact, a pending order or unresolved damage stands; demotion writes the summary before changing cadence; damage, orders, contacts and positions survive promote → demote → promote byte-identically. *Catch-up:* a journey's policy **preserves remaining IC duration** across a rate generation change (6A: journey clocks never pause — **deliberately not production's pause default**, D-E7-9); a long downtime does not teleport a carrier past an interception. *Restart:* a carrier mid-leg, mid-promotion and mid-demotion each hydrate; a promoted domain with no live `DomainTick` generation refuses to tick and pages.

**Verify lenses.** **T (topology)** — can any leg, promotion or crash orphan an occupant, a hold, a mount or a station? **S (state across cadence)** — can promotion or demotion lose, duplicate or reorder tactical state? **B (boundaries and limits)** — can a scene delay an arrival, or the active-domain limit be exceeded or silently swallow a trigger?

**Hands to cut 7:** a live domain with a stable `ClockRef::DomainTick` identity for propagation to run on, and journey legs for coarse scheduled delivery to run between.

### 3.7 Cut 7 — `mush-propagation`: one context, two resolvers

**4A/R4.** One crate owns **contacts and delivery envelopes over a shared physical propagation context, with separate resolvers**. 4C (both inside `mush-sim`) was cheaper; 4A wins because propagation has consumers with no spatial dependency (doc 10 §18.2).

```rust
struct PropagationContext { domain: Option<DomainId>, geometry: Geometry, medium: Medium,
                            interference: Interference, at: ClockStamp }
struct ContactRecord { id: ContactId, observer: ObserverRef,        // 16A: a platform or station,
                       subject: Option<SubjectRef>,                 //      never a side, never the world
                       first_seen: ClockStamp, last_seen: ClockStamp, confidence_band: BandId,
                       revealed_attributes: BTreeSet<AttrId>, provenance: Provenance, generation: u64 }
enum SubjectRef { Unit(ObjId), Unidentified, False { cause: FalseCause } }  // artefact | decoy | spoof
enum Provenance { Local, Reported { source: ObserverRef, age: i64, via: EnvelopeId } }
struct Envelope { id: EnvelopeId, issuer: ObjId, recipient: RecipientRef, payload_ref: PayloadRef,
                  revision: u32, state: EnvelopeState, path: Vec<HopRef> }
enum EnvelopeState { Queued, Transmitted, Delivered, Acknowledged, Expired, Superseded }
```

**One context, two resolvers — not one reachability step** (4A). They share geometry, medium, range, occlusion, interference and time-of-flight, and answer different questions. **Detection** resolves signature, emissions posture, medium, line of sight, sensor capability, noise floor and observer posture. **Delivery** resolves addressing, transmitter/receiver compatibility, routing, relays, encryption, jamming and store-and-forward. R4's two cases are *a jammed unit that still takes orders* and *a courier intercepted but never detected*. Contacts project as `PerceivedRef::Contact { provider, id, generation }` over the kernel's existing `ContactId` — vocabulary with no producer until now (`mush-core/src/action.rs:146-150`, `:655-669`, main).

**Contacts are local; sharing is a delivery** (16A). A transmitted report creates or updates the **recipient's own** record with source, age, confidence and provenance, so a stale relayed track is visibly stale and a false report is attributable. **There is no omniscient fused record anywhere.** **Unidentified** and **false** contacts exist from the start, because retrofitting them would change every consumer's assumption that a contact has a subject.

**Orders require delivery** (T15). Issuing revision *N* does not make it the recipient's active order. The recipient obeys the **newest valid revision it has actually received**; under ECM it continues its last order and fallback doctrine while the commander sees the replacement unacknowledged. On restored reachability, deliver the **newest valid superseding order**, not an obsolete backlog. **An acknowledgement is itself a delivery** that must reach the issuer (4A) — which is *why* the commander can see a replacement unacknowledged. Ordinary jamming blocks or delays; content corruption and deception require an explicit electronic-warfare result and are later.

**Cadence** (4A). Tactical propagation runs on the **domain tick** (cut 8). With no domain active, interplanetary mail, market reports and couriers run a **coarse scheduled delivery pass on a campaign cadence**.

**Commands.** `+contacts` · `+contact <id>` · `+order <recipient>=<order>` · `+orders` (issued, with envelope state) · `+orders/ack <id>` · `@prop/envelopes <subject>` · `@prop/pass` (staff).

**Tests (~28).** *Fog:* a projected contact never carries truth beyond `revealed_attributes`; a recursive leak check over every player- and AI-facing serialization finds no `ObjId` of an undetected unit (doc 09 Gate 3); one observer's record is invisible to another on the same side; a `False` contact renders identically to a real one; error messages do not disambiguate; **timing does not** (a detection miss costs the same work as a hit). *Provenance:* a relayed report carries source, age and `via`; relaying a relay increases age monotonically and bounds hops at `MAX_ENVELOPE_HOPS`; a report from a destroyed observer still renders with its age; a false report is attributable to its transmitter. *Delivery:* revision 3 delivered after 2 supersedes; revision 2 arriving after 3 is `Superseded` and never becomes active; a jammed recipient keeps its last delivered order and fallback doctrine; restored reachability delivers only the newest valid revision, not the backlog; expiry is idempotent. *Ack:* the issuer sees `Transmitted` until the ack itself arrives; an ack lost in transit leaves the order `Delivered` and the issuer uninformed — a probe asserts the commander's view says exactly that; a duplicate ack is a HIT. *Cadence:* with no domain active a courier resolves on the campaign pass; promoting a domain mid-flight moves that leg onto the domain tick without duplicating or dropping it. *Bounds:* `MAX_CONTACTS_PER_OBSERVER`, `CONTACT_TTL`, `ENVELOPE_TTL` and payload size refuse rather than grow. *Restart:* every envelope state hydrates; a `Transmitted` envelope in flight at restart is neither delivered twice nor lost.

**Verify lenses.** **K (knowledge leak)** — can any viewer, render, error or timing difference reveal an undetected unit or another observer's picture? **O (order lifecycle)** — can a stale order become active, a superseded one execute, or an ack be inferred without arriving? **B (bounds and restart)** — can contacts or envelopes grow without limit, or change state across a crash?

**Hands to cut 8:** the per-tick detection and delivery passes the pulse driver calls, and the `ContactRecord` tactical intents target.

### 3.8 Cut 8 — the minimal deterministic pulse driver

**12A, both halves.** **Ingress:** player commands and externally submitted tactical intents are **new authoritative state-family actions and take the prepared path**. K3 already proves the shape with the `brace` provider — typed state family (`crates/mush-tactics/src/module.rs:592-621`, main), early idempotency and checked revision (`:504-536`), guard revalidation (`:539-542`), authoritative-only preparation (`:544-581`), action + validator + provider registration (`:623-635`). Doc 09 K5 and K6 then land on a lane E7 is **already on**. 12C — a TacSim-private action path — is forbidden by doc 09 C2 and T2. **Resolution:** the pulse resolver does **not** create one K action per NPC, weapon or unit. It consumes the frozen set of accepted intents for the tick and produces **one deterministic domain-tick commit per domain per tick** — ordered deltas, RNG evidence, state hash — **bounded by the K3 bundle limits** (`crates/mush-core/src/authoritative.rs:27-38`, main). **That bound is the real cap on a domain's active unit count, and it is stated here**: `MAX_UNITS_PER_DOMAIN` (§7) is derived from those caps and pinned by a test (§9 q7).

```rust
enum DomainClock { Passive, Pulse { period_ms: u32, generation: u64 } }   // was Passive-only
struct DomainTickCommit { domain: DomainId, tick: u64, clock: ClockRef,   // ClockRef::DomainTick
                          accepted: Vec<IntentRef>, rng: ResolutionRecord,
                          deltas: Vec<UnitDelta>, facts: Vec<BoundaryFact>, state_hash: [u8; 32] }
struct StandingOrder { unit: ObjId, channel: ChannelId, value: OrderValue,
                       set_at: ClockStamp, by: ActorRef }                 // latest-order-wins
```

**2A's split.** Heading, thrust, speed and sensor posture **persist until changed** (continuous, latest-order-wins); fire, launch, jump, dock, eject and damage control are **bounded discrete intents**. Players command rather than spam movement verbs; crew stations divide control; NPC and link-loss controllers use **the same legal actions and information**; rules resolve from recorded intents and RNG, allowing replay. Pulse periods: ground/atmosphere ≈ 1–2 s, space ≈ 5–10 s (§7 picks 2 s and 6 s). A turn-driven mode remains for tutorials and facilitated scenes.

**T2, enforced at the seam.** **No economic transaction occurs inside a combat pulse.** A tick mutates combat state (ammunition, heat, damage, position, control) over `TacticalUnit`'s existing cells, sections and installed components (`crates/mush-tactics/src/unit.rs:220-245`, `:314-326`, `:328-352`, main) and emits **typed facts at boundaries only**: sortie started/ended; unit disabled, abandoned, captured, destroyed; cargo delivered; docking completed; salvage recovered. Combat RNG belongs in the mechanical journal, not the economic one. **A probe asserts no economy entry point is reachable from the tick handler.** Every accepted intent records actor, decision driver, authority and control-lease epoch (T9) and revalidates its exact station/channel/epoch through K5a (`world.rs:684-699`, K) before it resolves.

**Commands.** `+fire <contact>` · `+launch <bay>=<contact>` · `+heading <vec>` · `+thrust <n>` · `+posture <emcon>` · `+dc <section>` · `+dock <target>` · `+eject` · `@domain/tick <id>` (staff, single-step) · `@domain/replay <id> <from>-<to>` (staff).

**Tests (~28).** *Determinism:* the same accepted-intent set and RNG root produce byte-identical deltas and `state_hash` across two runs and a restart; replay from a stored tick reproduces the stored hash; intent ordering is canonical and independent of arrival order within the tick. *Bounds:* a domain at `MAX_UNITS_PER_DOMAIN` commits inside the K3 caps; one unit past it refuses promotion (or queues, per 15A) rather than producing an oversized bundle; a probe measures encoded tick bytes and fails past a stated fraction of the cap; retained-intent capacity bounds accepted intents per tick. *Ingress:* an intent from a non-focused station refuses (cut 5); a stale epoch refuses; a duplicate idempotency key is a HIT; a channel the grant does not carry refuses; an intent naming a contact the actor does not hold refuses **without revealing whether it exists** (cut 7). *T2:* no tick path reaches an economy entry point (probe); a sortie boundary emits the facts and *then* the economy consumes them; a settlement attempted inside a tick refuses; the sortie cadence advance happens at the boundary, once. *Continuous vs discrete:* a second heading order in one tick replaces the first and both are journaled; two `+fire` in one tick are two bounded intents; an order superseded before resolution leaves only the newest active. *Restart:* a crash mid-tick converges — the tick committed whole or not at all, and re-running produces the same hash; accepted intents survive; the RNG stream does not re-draw (K3.1a). *Fallback:* a linkdead station's declared NPC fallback issues legal actions from the same information (C30A) and is recorded with its own driver attribution.

**Verify lenses.** **D (determinism and replay)** — can two runs of one tick differ, or a restart change a result? **C (commit bounds)** — can a tick produce a bundle the store must refuse, and does the unit cap actually hold? **X (the T2 boundary)** — can any economic effect, RNG draw or settlement cross into a pulse?

**Hands to cut 9:** boundary facts with stable ids, and a domain an operation's activation can promote.

### 3.9 Cut 9 — `mush-campaign`: declared operations, formations, resolution

**5D.** A **new neutral crate**; one logical operation, **separately versioned terms and runtime under one `OperationId`**. R1 is satisfied by the shared id and the one-to-one link without conflating consent with runtime. 5A is rejected because it re-learns the E4b2 lesson: every roster edit becomes either a terms amendment or a hole in the hash.

```rust
struct OperationTerms {                      // FROZEN at activation (T6)
    operation: OperationId, objective: ObjectiveSpec, legal_targets: Vec<TargetClass>,
    stakes: Vec<Consequence>, bond: HoldId, sides: Vec<SideSpec>,
    eligible_defenders: AudienceSpec, notice: ClockStamp, window: (ClockStamp, ClockStamp),
    visibility: EconomicVisibility, consent_hash: [u8; 32],   // ← mush-economy's helper, shared
}
struct OperationRuntime {                    // MUTABLE
    operation: OperationId, participants: Vec<Participation>, force: TaskForce,
    reinforcements: Vec<UnitRef>, withdrawals: Vec<UnitRef>, casualties: Vec<UnitRef>,
    handoffs: Vec<SeatHandoff>, progress: Progress, state: OperationState, revision: u64,
}
enum OperationState { Declared, Activated, Running,
                      Resolving { pending: BTreeSet<ConsumerId> }, Resolved, Aborted }
struct FormationKind { id: CanonicalId, branch: CanonicalId, echelon: u8,
                       composition: Composition, command_roles: Vec<RoleSpec> }
struct Composition { minimum: u8, maximum: u8, preferred: u8,        // `exact` ≡ minimum == maximum
                     allowed: BTreeSet<CategoryId>, required_command: Vec<RoleSpec>,
                     required_support: Vec<RoleSpec> }
struct FormationNode { id: FormationId, kind: CanonicalId, scope: OrgScopeId,   // exactly one (R2)
                       parent: Option<FormationId>, members: Vec<UnitRef> }
```

**Terms freeze; the roster does not touch them.** **Changing the roster never changes the terms hash** — a reinforcement is not an amendment to what the defenders consented to be told. The hash uses **`mush-economy`'s existing helper**, shared not forked (`contract.rs:999-1030`, signature `:1014`, main; 10a §6 sharpening 3), and `consent_intact` (`:1063-1077`) is re-checked at every transition.

**T6's visibility classes.** `EconomicVisibility::Audience { members, announce_issuer }` with `announce_issuer` defaulting to `false` (`record.rs:339-365`, `:362-363`, main) is already T6's *"notice of the window, not the plan"*: public; eligible defenders; participating organizations; covert existence with a visible vulnerability warning; staff/audit only. **"Declared" means every party entitled to defend receives sufficient notice and knows the window — not that the attacker's plan or identity is published.** `MAX_AUDIENCE` is 32 (`record.rs:322-323`).

**The bond** is an economy hold placed by the attacker's **own** authority — never an industry reservation, because it is money (T4). 5A's ≈ 24–72 h notice is `OPERATION_NOTICE_MIN/MAX` (§7).

**Formations** (9A/T8/R1/R2). `composition` is a **constraint set, not one enum**, and **`exact` is `minimum == maximum`**, not a separate mode. **Strictness is scoped** — at *creation*, at *doctrine qualification* ("is this still a full squadron?"), or as *an operation's requirement* — **never to a formation's existence or its ability to deploy**. An understrength permanent formation is normal, not an error. `ruleset-tacsim` authors the instances in cut 1's packs, so a four-unit lance and a five-unit star share an echelon and remain distinct kinds. **Each node names exactly one owning `OrgScopeId`** and `force.*` capabilities resolve through the existing grant machinery (`crates/mush-org/src/authorize.rs:143-200`, main), so **R2 needs no core change**. It does need one composition-root addition: `org_capability_vocabulary()` is **non-empty** and is a typo catcher at the grant command (`crates/mush-server/src/modules.rs:176-182`, main, holds only economy, housing and two org ids; `crates/mush-org/src/wiring.rs:78-93`), so granting an unlisted `force.*` id **fails loudly at `@org/grant`** — the ids must be added there. Administrative parentage is unchanged by attachment; **a unit is in at most one operation at a time** (R1), with exactly one operational command parent within it.

**Terminal settlement** (17A). An operation ends by writing **one** `campaign.operation.resolved` root fact into the mechanical journal (T2). Every consequence is a separate, **idempotent**, causally linked consumer transition keyed off that root: campaign outcome recording; front and control mutation through `mush-faction` (`sim.rs:65-98`; the flip at `lib.rs:235-274`, main); bond settlement; contract outcome selection (`contract.rs:227-276`, main); task-force retirement. The operation stays **`Resolving` until every required consumer acknowledges**, and only then reaches `Resolved`, **so a crash mid-fan-out resumes rather than double-pays**. 17B is rejected: it claims an atomicity the tree does not have across three crates (08c §12).

**Commands.** `@op/declare <objective>` · `@op/terms <id>/<field>=<value>` (pre-activation only) · `@op/activate <id>` · `@op/join <id>=<unit|formation>` · `@op/withdraw <id>=<unit>` · `@op/show <id>` (per audience) · `@op/resolve <id>=<outcome>` · `@op/consumers <id>` (staff) · `@force/create|attach|detach|assign` · `@formation/show <id>`.

**Tests (~30).** *Freeze:* a terms edit after activation refuses; a roster edit leaves `consent_hash` byte-identical; a mutated surface makes `consent_intact` false and the operation refuses to progress — it disputes and pages rather than settling; the notice window sits inside `OPERATION_NOTICE_MIN/MAX`. *Audience:* a non-audience viewer sees no operation id, issuer or objective (the existence-hiding renderer treats it like an account id); an audience member sees the window and notice but **not** the plan when `announce_issuer` is false; a recursive leak check finds no covert operation's existence; an audience over `MAX_AUDIENCE` refuses. *One operation:* a unit already in an operation refuses a second; withdrawal releases it; a formation attached keeps its administrative parent; a node with no `OrgScopeId` is unconstructible; a `force.*` capability missing from the vocabulary fails at registration, not silently at use. *Composition:* an understrength formation exists and deploys; the same constraint set refuses at creation and at doctrine qualification; `minimum == maximum` behaves as `exact`; a lance and a star share an echelon and stay distinct kinds. *Bond:* the bond is a `Hold` placed by the attacker's own authority; an attacker without funds cannot activate; it settles exactly once per outcome row. *Resolution:* the root fact is written once; each consumer transition is idempotent under replay; a crash after two of five consumers resumes and does not double-pay; `Resolved` is unreachable while any consumer is pending; a permanently failing consumer opens a level-2 escalation with the recommended command filled in (08c §4.6) and **suspends only the operation**. *Fronts:* a resolution advancing a front writes one `ControlRecord` change, and re-running the consumer writes none.

**Verify lenses.** **Z (freeze integrity)** — can any path change what a defender consented to be told, or progress an operation whose surface no longer hashes? **A (audience)** — can a viewer learn a covert operation exists, or its plan, through any render or error? **I (idempotent fan-out)** — can a crash, retry or duplicate root fact double-apply a consequence, or strand an operation in `Resolving`?

**Hands to cut 10:** the operation scope seat grants, musters and readiness obligations attach to, and the participation runtime settlement indexes on.

### 3.10 Cut 10 — crew, proxy seats, musters, the oracle

**Crew** (C11A/C13A/C14A/C15A/T16). Named NPC objects occupy dramatically relevant positions — captain, helm, tactical, engineering, marine command; routine watchstanders, technicians, loaders, stewards and medical staff are an **aggregate complement** carrying readiness, capacity and morale. A **periodic readiness obligation** covers ordinary payroll and support — an E4b1 `Obligation` on the **operation cadence with success notifications off** (doc 10 §15) — and individual employment contracts are used only when a named NPC's personal terms matter. **T16:** the actor at a station supplies the relevant traits, and **a human proxy directing an NPC still uses the NPC's sheet** — the human adds judgment, not an absent character's stats. Any wider effect needs a causal action or changed ship state, **never a free global bonus for seat occupancy**. Crew quarters reuse E5a's tenancy shape (08d §3.5), not a new one.

**Proxy seats are consent grants, not contracts** (10D; doc 10 R6 **as amended**). The governing fact: **an OOC proxy player is not an in-world principal.** C1A/R3 forbid an account principal and keep it out of any contract or canon record, and T13 says the absent character did not take the station — **so an E4 contract can name neither, and there is no third candidate.** 10A is rejected for exactly that reason.

```rust
struct SeatOffer { id: SeatOfferId, operation: OperationId, audience: AudienceSpec,
                   eligibility: EligibilitySpec, expires: ClockStamp, scope: SeatScope,
                   revocation: RevocationTerms, fallback: FallbackSpec, consent_hash: [u8; 32] }
struct SeatScope { unit: ObjId, station: StationId, channels: BTreeSet<ChannelId>,
                   mission_order: OrderRef, roe: RoeRef, permitted_area: AreaRef }
struct SeatGrant { offer: SeatOfferId, actor: ObjId /* the NPC */, driver: DriverToken,
                   accepted_at: ClockStamp, consent_hash: [u8; 32], state: GrantState }
enum GrantState { Dormant, Active { epoch: u64 }, Revoked, Expired }
```

**T11's guardrails are the granted channel set on the NPC's `StationSpec`** (`mush-sim/src/data.rs:414-435`, channels at `:426`, main), **not contract prose**. Proxy control grants tactical discretion, **not** title, administration or an unrestricted possession verb: no asset transfer or abandonment, scuttling, fallback change, loadout or crew mutation, organization spending, surrender of unrelated assets, departure from the authorized operation, or assumption of another station. **Ordinary tactical loss cannot be made impossible without making the seat fictitious.**

**Live control is a lease derived from the grant, not the grant itself.** Revocation bumps the unit's control epoch immediately (`mush-sim/src/data.rs:615-622`, main) so stale intents fail the K5a check, and the seat returns to its declared fallback at a safe boundary (T11). **K6 gates the actual handoff** (§2.4): this cut ships **grants, fallback and dual attribution** and **stubs the live handoff behind the gate** — it may not silently evict a live operator or transfer an active movement/weapons lease. An **E4 contract links to the operation only when an actual fictional principal** — embodied character, named NPC, organization — **has economic terms**; an unpaid proxy seat needs none, and inventing one to hold the consent hash is 10A's error.

**Attribution and audit** (19A/T9/R7). The mechanical journal records the **in-world actor** and an **opaque decision-driver token**, and nothing else. A **server-side, audit-only** record links that token to the account; ordinary projections, renders and exports cannot reverse it, and one staff verb can. OOC participation history and matchmaking standing **consume the audit record**, live outside the world, and **never become IC settlement**: T13's absent character earns **no** pay, advancement, reputation, knowledge or property, and standing is a **crew-call eligibility policy input only** (R6).

**Focus** (20A/T12). OOC proxy control **consumes the account's one foreground focus** (cut 5). The player's own character remains at their durable location and **cannot act concurrently**; entering proxy mode hands off that character's station, clearly identifies the represented NPC and exposes **only that NPC station's observer projection** (cut 7). Leaving restores AI control; it never moves or protects the player's character, who is present and vulnerable under E5's attention budget and non-response semantics (C22A/R5). **Accepting an offer does not seize focus** — the grant is `Dormant` until activated.

**Musters** (T14/21A). E7 uses **E6a's `mush-party`** — game-neutral, `{mush-core}` deps only, not a principal, never learning about money (08f §2). **There is never a TacSim-specific party record**; if E7 lands first it pulls that exact cut forward unchanged. An operation publishes start, expected duration, committed formations, character billets, proxy-eligible NPC stations, certifications and restrictions, command succession, secrecy and compensation. At muster a slot resolves to a player character, a human-directed NPC, an AI-controlled named NPC, automation, or vacancy. **Missing players reduce discretion and resilience but do not cancel a valid operation.** Late arrival and early departure use the same explicit handoff (C29A).

**The distinctness oracle's consumers** (11A/R3/C23A/C24A). Roster lock and side assignment call `all_accounts_distinct` and **store only the verdict**; the private account id never enters a journal, contract or canon record. An account may materially assist **only one side** of a defined operation, battle, claim contest or similar competitive episode.

**Participation facts** (R6/T13; doc 10 §15). Settlement splits index on **participation**, which E4b2's outcome-indexed terms already support once the sim emits `sim.station.served { actor, station, from, to }`, answered from an **applied marker, never a status** (08c §6.2; 08f §7 item 3). Kill, salvage and delivery credit are the same shape.

**Commands.** `+seat/offer <operation>/<unit>/<station>` · `+seat/list` · `+seat/accept <id>` · `+seat/decline <id>` · `+seat/activate <id>` · `+seat/leave` · `@seat/revoke <id>` · `+muster <operation>` · `+crewcall/list|apply` · `@crew/complement <unit>` · `@crew/readiness <unit>` · `@seat/audit <token>` (staff only).

**Tests (~30).** *Guardrails:* each of T11's nine forbidden acts refuses from a proxy seat and succeeds from the owner; a granted channel set narrower than the station's limits the proxy exactly; a proxy cannot assume a second station; an intent outside `permitted_area` refuses; **ordinary tactical loss is permitted** — a probe asserts the seat is not made safe. *Revocation:* revocation bumps the epoch and every queued human, NPC, pathfinder and LLM intent under the old grant fails stale; the seat returns to its declared fallback **at a safe boundary**, not mid-tick; a disconnect uses the same path; a revoked grant cannot be re-activated. *K6 gate:* activating a grant over a **live** operator refuses with the gate named, and a test pins that E7 ships no eviction path. *Focus:* activating a seat releases the player's own station's control and fires its fallback; the player's character stays where it was, present and vulnerable, and cannot act; leaving restores AI control and does not move the character; accepting without activating seizes nothing. *Attribution:* the journal carries actor + driver token and **never** an account id (a recursive scan of journal bytes asserts it); the audit record resolves the token for staff and nobody else; an export cannot reverse it. *Absent-player settlement:* the absent character receives no pay, advancement, reputation, knowledge or property from a proxy sortie; canon attaches to the NPC, unit, organization and operation; matchmaking standing changes and **cannot** be spent on character power. *Distinctness:* two characters on one account fail `all_accounts_distinct` at roster lock and side assignment; an oracle answering `None` fails closed; only the verdict is stored. *Musters:* a slot resolving to vacancy does not cancel the operation; a late arrival takes a seat through the same handoff; a probe asserts no TacSim party type exists. *Readiness:* the obligation runs on the operation cadence, is silent on success, and its arrears follow E4b1's ladder.

**Verify lenses.** **G (guardrail escape)** — can a proxy reach any authority outside its channel set, area or operation? **R (revocation and boundary)** — can control persist past a revocation, be lost mid-tick, or be handed off before the boundary? **A (audit irreversibility)** — can the account behind a driver token be recovered by anything but the staff verb, or reach a journal, contract or canon record?

**Hands to cut 11:** a crewed, operationally committed force the proving loop can run, and the participation facts settlement indexes on.

### 3.11 Cut 11 — the recovery provider, the corridor seed pack, the rate policy

**The recovery provider** (7A/T7; doc 10 3A). **No new engine machinery.** The provider is a **government or corporate organization principal** owning the loaner inventory, accounts, contracts and policy. A system faucet may **subsidize** it under the `[economy]` mint cap via `create_system_obligation` (`crates/mush-economy/src/api.rs:1105-1203`, main) — **a `System` principal can never itself be a contract party** (08f D-E6-4, which the tree forces). **Eligibility is a deterministic predicate over recorded facts examining accessible operational assets** — unencumbered, reachable, deployable — **rather than total title**, so a player whose only hull is an unreachable wreck still qualifies. **Staff are never the gate.** The grant is a contract placing `Encumbrance::Lease` on the loaner (`record.rs:528-536`, main), with upkeep an `Obligation` on the campaign cadence and restrictions as ordinary conditions. 7A requires the plan to state five things, and it does:

| Question (7A) | Answer |
|---|---|
| Pool exhaustion | the queue holds; the provider stops granting and pages **level 1** — an empty pool is not a decision |
| Queue priority | first-eligible, then longest-waiting; ties by claim time |
| Per-player cooldown | `RECOVERY_COOLDOWN` after a lease ends, waived when the loss was inside the restrictions |
| Insurance | approved operations only; outside them the lease is uninsured and a loss becomes a debt |
| Replenishment | the provider's own production work orders (cut 4) and salvage recovery, funded by its budget — **never a mint into the pool** |
| Upgrade / loss | buy out at a declared price or re-lease one tier up; loss inside the restrictions is written off, outside it becomes a debt |

**The corridor seed pack** (6A/18A/§11). The full Mars–Phobos–Ceres content as **data**: propulsion and route tables, the ten commodity families, component definitions, recipes, formation kinds, combat formulas, insurance and warfare policy in **rules-definition packs**; the corridor locations, the player corporation, the government faction and its rival, the refinery, the mining drones, the hauler, the patrol craft, the dropship, the infantry, the tank and the mech in a **campaign-seed pack**. **No hot mutation** (18A): a change publishes a new version that coexists, and live units, blueprints, recipes and open contracts stay **pinned** until an explicit migration or in-world refit — **also T1's guarantee that a published hauling contract's route assumptions cannot shift underneath it.** The market is **direct vendor sale only**; E7 builds no order book (§1.1).

**The timeline's rate policy** (8D). **The kernel owns generations** (K4a, §2.1); **the campaign layer sets rate policy** — doc 10 1A's ≈ one OOC hour per IC day is a **policy value, not a constant in code**. Every subsystem registers a **derived cadence adapter** over the one timeline. **Multiple named cadences are fine; multiple incompatible real→IC mappings are the danger** — WoD's and Shadowrun's clocks stay separate `CadenceId`s and never derive TacSim's rate, so a compression change cannot re-price a Shadowrun lifestyle (doc 10 §20). `catch_up_cap` stays **obligation-sweep policy** (`mush-economy/src/clock.rs:240-241`, `:264-295`, main); E7 gives **journeys, production, fronts and tactical domains each an explicit catch-up policy**, because they are not obligations:

| Subsystem | Catch-up policy on a new generation | Grounding |
|---|---|---|
| Journeys | **restamp preserving remaining IC duration** — journey clocks never pause | 6A |
| Production | **pause as `Interrupted { TimelineChanged }`** unless the campaign registers a restamp | proposal §5; D-E7-8 |
| Fronts | restamp preserving the real due instant — a front clock is real-time pressure | 5A; `mush-faction/src/sim.rs:65-98` |
| Tactical domains | **refuse to tick and page** — a live domain under an unknown generation is an integrity incident | 12A; doc 09 §2.5 |

**Commands.** `@recovery/status` · `@recovery/apply` · `@recovery/grant <who>=<tier>` · `@recovery/return <lease>` · `@tacsim/time show` · `@tacsim/time rate=<n>` (campaign authority — opens a generation) · `@tacsim/time policy <subsystem>=<policy>`.

**Tests (~28).** *Eligibility:* an unreachable wreck does not disqualify; an unencumbered deployable hull does; a holder of a live loaner is refused a replacement, naming the lease; the predicate reads only recorded facts and never a staff flag. *Provider:* an exhausted pool queues and pages level 1; the provider's budget is exhaustible and it stops granting when dry (doc 10 7A); replenishment comes only from production and salvage, and a probe asserts no path mints into the pool; a `System` payer is refused (D-E6-4). *Lease:* the loaner carries `Encumbrance::Lease` and cannot be sold or transferred; upkeep runs on the campaign cadence; a loss inside the restrictions writes off and outside creates a debt; a buyout clears the encumbrance once. *Packs:* publishing `v2` leaves every `v1`-pinned unit, blueprint, recipe and open contract unchanged; a live hauling contract's route assumptions do not shift (T1); a migration is explicit and attributed; a refit re-pins one subject; digest stability holds across restart. *Rate policy:* opening a generation is one attributed act; every subsystem's declared policy fires exactly once per generation; a journey does not pause; production pauses with no registered restamp; a tactical domain refuses to tick and pages; a Shadowrun `sr.month` cadence is unchanged by a TacSim rate change; `catch_up_cap` still governs the obligation sweep only. *The proving loop:* the §11 story runs end to end on the seed pack — extract, haul, intercept, manufacture, load, deploy, fight, settle, salvage, repair, pay — with the ground engagement and the local-space interception both resolving through cut 8 and settling through `campaign.operation.resolved`.

**Verify lenses.** **L (the loop)** — does the §11 story complete without a staff step, and does every arrow leave a recorded fact? **V (versioning)** — can a pack change reprice, re-time or re-resolve anything already created or contracted? **F (faucet and floor)** — can the recovery provider be drained into an uncapped faucet, or refuse a player the floor 3A guarantees?

---

## 4. Build discipline

As 08c §7 and 08a's standing rhythm, per cut: (1) **confirm the cut's slice of this plan against the tree** — both trees, since E7 rebases onto K; (2) build, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; (3) **a probe-confirming adversarial verify of that cut** — the lenses named in its section, throwaway probe tests — **before the next cut builds on it**; (4) fold every confirmed finding with a regression test named `vf_<lens><n>_…`; (5) commit with the verify outcome in the message; **push only on the owner's word**. Both digest pins (E2 `9c917eee…`, E3-shaped `b780e3a0…`) stay green in every cut, and **`cargo fmt` stays off** (08a).

**The production-slice proposal's §9 review checklist is adopted as E7's standing verify checklist**, quoted in full and applied to every cut:

> - No command or timer can confirm before its durable acknowledgement.
> - No `?` or other fallible operation remains after the first irreversible live mutation.
> - Every retryable entry has a bounded scoped idempotency key and semantic request hash.
> - Holds subtract from availability and facility capacity; embedded advisory counters are not authority.
> - Rate changes create generations; old facts retain old generations.
> - Channels and priorities are never unioned across station grants.
> - Title/custody/location/station/grant/lease/maintenance remain separate facts.
> - Completion cannot expose an object before all required owner projections agree.
> - All lists, catch-up loops, recipes, inputs, outputs, reservations, events, and DTO bytes are bounded.
> - Staff intervention is the last resort after deterministic reconciliation and scoped recovery have been exhausted; any page includes the exact order, failed invariant, and recommended runbook.

The last item is 08c §4.6's ladder in the proposal's words, and both are binding: **resolve programmatically wherever the best action is determinable; page a human only for a genuine decision; stop only the subject; never stop the world for a local problem.** The sixth item is why §2.2's main-vs-K difference is a rebase blocker rather than a preference.

---

## 5. Kernel migration — a later cut, "E7k"

The proposal's **E7k** milestone is retained as E7's twelfth, later cut. It does **not** ship with cuts 1–11 and **nothing in them blocks on it** (10a §4 dep 2).

**The gate.** All three §2.4 items must land first: mixed authoritative write sets including the Ledger append; synchronous typed-family module reads outside an action; the Ledger seal folded into the same exact bundle. **Then a migration review opens.** 3A is explicit — *the trigger opens a review, never an automatic migration* — and 08c §12 says of the equivalent move that it "is then still an explicit **owner decision**, not a mechanical port" (`08c:1810-1813`, main), **because the journal would stop being sole authority for money-adjacent truth, which is the property every economy invariant is anchored to.**

**What would move:** the cut-2 inventory reservations and cut-4 capacity reservations from `mush-inventory`/`mush-industry` documents onto authoritative K3 state families; work-order transitions; the Ledger append accompanying a settlement; and cross-owner output materialization (title + custody + placement + inventory in one bundle). Cut 9's resolution fan-out is a candidate too — 17A adopts a truly atomic mixed bundle **only when the kernel provides one**.

**Until then** the economy journal remains authority and E7's conventional documents are projections or workflow records. **Every record in cuts 2–4 and 9 is shaped for a mechanical migration** (proposal §2.3, adopted verbatim as a build rule): canonical whole-document post-images, one mutation per key per transition, explicit revisions, deterministic idempotency keys, and no hidden side effects after the final fallible preflight.

**What must prove equivalence before the conventional handler is removed** (proposal §8) — the **memory and Redis restart matrix** and the **acknowledgement-loss matrix**, both against both stores: restart after commit / before RAM install loads the committed post-state; restart after ack / before output recovers the receipt and keyframe; a lost ownership ack converges by replaying the same request UID; acknowledgement loss cannot duplicate state or re-run RNG; an action-created object survives allocator restart **without id reuse**; a partial materialization always has a complete exact pending intent; legacy pre-state dependencies cannot be lost beneath a later committed action (doc 09 K3 exit criteria, `09:2126-2134` *(K)*). Redis-gated tests are explicit ignored tests locally **and** an explicit Linux CI job, so skipped coverage cannot count as passing (K3.1e, `09:2075-2083` *(K)*).

---

## 6. Decisions

Each cites the 10a answer, doc 10 item, or `file:line` it rests on. **Ov** marks an override of the production-slice proposal.

- **D-E7-1** E7 is **one coherent narrow vertical slice** — one ground engagement and one local-space interception with authored units; construction balance, the weapon-family matrix, boarding, aerospace transitions and fleet tuning are later (**22A**; doc 10 §11). **Ov:** the proposal's outcome is "one generic production flow"; production is cuts 2–4 of eleven, not the whole of E7.
- **D-E7-2** **One C8-conformant reservation shape, two owners, and money is neither.** Doc 09 C8's exact record (`09:997-1008`, main) is used for **matter** in `mush-inventory` (13A) and **capacity** in `mush-industry` (3A), each registering its own `AuthorityGuardKind::Reservation` validator under its own `OwnerId` (`action.rs:1417-1516`, `:1534-1561`, main). **Money stays an E4 `Hold` and never grows a resource field** (T4; doc 10 §15). **Ov:** the proposal lists money, material and capacity as undifferentiated "reservations" (§4).
- **D-E7-3** **`mush-inventory` is the single source of quantity truth**, location-bound to E3 custody endpoints; industry, markets, contracts and simulation consume it and **none keeps a private stock number** (**13A/2D**; `custody.rs:61-131`, main). **Ov:** the proposal's "material vault" quantities and `material_source: AssetContainerRef` become an `EndpointRef`.
- **D-E7-4** A lot carries `{ id, commodity, quantity, grade, lineage }` — **not `legal`, not `custody`** (**2D**). Custody is the endpoint's; stolen/disputed follows the **title** record (`record.rs:521-527`, main); embargo legality is a **jurisdiction test at transaction time**; licensing is the **acting principal's** entitlement.
- **D-E7-5** Interconversion is **one journaled attributed act each way**; **lineage is never discarded, only coarsened in the projection** (**2D**; T5).
- **D-E7-6** Yield is a **deterministic curve with depletion**; variation is discrete campaign events, not per-period rolls; a survey reveals **bands**, never the exact reserve (**1A**; doc 10 4A).
- **D-E7-7** **Claims are records; licences are entitlements** — transferable, pledgeable, contestable, suspendable, seizable; a licence is an `InstrumentKind::Entitlement` account checked at transaction time (**14A/1A**; `record.rs:50-61`, main).
- **D-E7-8** The **default clock-generation policy is pause as `Interrupted { TimelineChanged }`, and the *campaign* registers a restamp** (proposal §5, adopted; **8D**). **Ov:** the proposal's §3 gives the ruleset "clock choice"; 8D reserves generations to the kernel and rate policy to the campaign layer.
- **D-E7-9** **Every subsystem declares an explicit catch-up policy**, because journeys, production, fronts and tactical domains are not obligations; `catch_up_cap` stays obligation-sweep policy (**8D**; `clock.rs:240-241`, main). Journeys **never pause** (doc 10 6A) — deliberately the opposite of production's default.
- **D-E7-10** **One foreground focus per account, atomically acquired, with a declared fallback on release**; multiple connections attach to one focus (**20A**; C21A/T12).
- **D-E7-11** **Billets are eligibility; only the focused station's `ChannelGrant` is live; channels and priority are never unioned across seats** (**C2A**; K5a at `crates/mush-sim/src/links.rs:18-59` and `crates/mush-core/src/world.rs:50-69` *(K)*). **E7 rebases onto K:** main still unions (`links.rs:39-54`), the hazard doc 09 names at `09:989-993`.
- **D-E7-12** The **carrier-root invariant replaces the traveler-unplaced rule** (doc 10 **§16 item 9**/C16A; `route.rs:67-90`, `commands.rs:1529-1564`, `data.rs:678-690`, main).
- **D-E7-13** Domains **promote on four deterministic triggers and demote only at a quiet boundary after a durable summary**, under a hard `MAX_ACTIVE_DOMAINS`; **state is preserved both ways** (**15A**).
- **D-E7-14** **One propagation layer, one physical context, two resolvers; an acknowledgement is itself a delivery** (**4A**; R4).
- **D-E7-15** **Contacts belong to a platform or station, never a side or the world**; sharing is a delivery carrying source, age, confidence and provenance; unidentified and false contacts exist from the start (**16A**).
- **D-E7-16** **Tactical ingress takes the prepared K lane from the first pulse; resolution is one deterministic domain-tick commit per domain per tick, bounded by the K3 bundle limits** — the real cap on active unit count (**12A**; `authoritative.rs:27-38`, `mush-tactics/src/module.rs:470-635`, main). **Ov:** the proposal's E7e binds every mutation to K5 legal offers; K5 has not landed and no cut waits for it.
- **D-E7-17** **No economic transaction occurs inside a combat pulse**; a tick emits typed boundary facts the economy consumes afterward; combat RNG stays in the mechanical journal (**T2**).
- **D-E7-18** An operation is **one `OperationId` with separately versioned frozen `OperationTerms` and mutable `OperationRuntime`; a roster edit never changes the terms hash** (**5D**; R1), using **`mush-economy`'s existing `consent_hash`**, shared not forked (`contract.rs:999-1030`, main; 10a §6 sharpening 3).
- **D-E7-19** **Formation composition is a constraint set with `exact ≡ minimum == maximum`, strictness scoped to creation, doctrine qualification, or an operation's requirement — never to existence or deployment** (**9A**; T8). Each node names one `OrgScopeId` (R2). R2 needs no **core** change; it does need the `force.*` ids added to `org_capability_vocabulary()` (`modules.rs:176-182`, main), since that non-empty set is a typo catcher and granting an unlisted id fails at `@org/grant` (`wiring.rs:78-93`).
- **D-E7-20** An operation resolves through **one journaled root fact and idempotent consumer transitions, staying `Resolving` until every consumer acknowledges** (**17A**); a truly atomic mixed bundle waits for the kernel (08c §12).
- **D-E7-21** **A proxy seat is a consent grant, not an E4 contract**; live control is a lease derived from it; **K6 gates the actual handoff**, so cut 10 ships grants, fallback and dual attribution and stubs the handoff (**10D**; doc 10 R6 as amended; proposal §2.4; `09:2151-2153` *(K)*).
- **D-E7-22** **The account id is opaque and audit-only**; the only exposure is a boolean distinctness oracle and one staff verb; it never enters a journaled, contract or canon record (**11A/19A**; R3/R7). Its two prerequisites — an immutable id on `AccountRec` and a boot-hydrated character→account map — ship in cut 5 (`mush-store/src/types.rs:718-741`, main, has neither).
- **D-E7-23** The **recovery provider is a funded organization**, eligibility tests *accessible* assets, and a `System` principal is never a party (**7A**; T7; 08f D-E6-4). Staff are never the gate.
- **D-E7-24** **Packs are immutable under their digest; versions coexist and consumers stay pinned** until an explicit migration or in-world refit (**6A/18A**; T1; `mush-sim/src/data.rs:227-232`, `mush-core/src/module.rs:622-634`, main).
- **D-E7-25** **The eleven cuts of 10a §4 are E7's structure**; the proposal's milestones **E7a–E7e** fold into cuts 2–4 and its **E7k** becomes §5. **Ov**, explicitly.
- **D-E7-26** **E7 builds no market.** Direct vendor sale carries the physical loop; `mush-market` and the safe public projection stay **E6c's**, and §11's "one open market" is incomplete until E6c (10a §4 dep 1, §5).
- **D-E7-27** **No E7 cut blocks on a K item** — the C8 interim is K-independent and the sim-side staleness check is already the control epoch (`mush-sim/src/data.rs:635-640`, main); the only K dependency is the migration **review** (**3A**). E7 does **rebase** onto K for K4a/K5a.
- **D-E7-28** **Crew quarters reuse E5a's tenancy shape** rather than a second one; `Ended` stays reachable only through a `Resolution` (08d §3.5, D-E5-1).
- **D-E7-29** **A route is a pack-authored `RouteDef`** (cut 1), because none exists: a route is only `<room>[:secs]` tokens parsed in `route_create` today (`crates/mush-sim/src/commands.rs:1584-1648`, main), which T1's frozen route assumptions cannot rest on.
- **D-E7-30** **`mush-sim`, `mush-tactics` and `mush-campaign` each register exactly one `FactProvider` namespace** — none exists today, so every `sim.*`/`tactics.*` `Fact` condition answers `None` (`facts.rs:35-40`, `:62-81`; `modules.rs:148`, main), and `with_facts` panics on a duplicate namespace (`module.rs:607-629`).

---

## 7. Constants

`MAX_ACTIVE_DOMAINS` 8 · `GROUND_PULSE_MS` 2_000 · `SPACE_PULSE_MS` 6_000 (inside 2A's 1–2 s / 5–10 s bands) · `MAX_UNITS_PER_DOMAIN` 64 (derived from `authoritative.rs:27-38` and pinned by a test, §9 q7) · `QUIET_BOUNDARY` 10 ticks · `MAX_INTENTS_PER_TICK` 256 (the durable-outbox intent cap) · `MAX_PACK_ENTRIES` 512 · `MAX_SEED_OBJECTS` 2_048 · `MAX_LOT_PARENTS` 16 (E6's, 08f §9) · `MAX_LINEAGE_DEPTH` 32 · `MAX_POOLS_PER_ENDPOINT` 64 · `MAX_WORK_ORDER_INPUTS` 16 · `MAX_RESERVATIONS_PER_ORDER` 12 · `MAX_FACILITY_SLOTS` 8 · `RESERVATION_TTL` 7 IC days · `MAX_SURVEY_BANDS` 8 · `MAX_CLAIMS_PER_SUBJECT` 8 · `MAX_CONTACTS_PER_OBSERVER` 128 · `CONTACT_TTL` 30 ticks past `last_seen` · `MAX_ENVELOPE_HOPS` 4 · `ENVELOPE_TTL` 30 IC days · `MAX_FORMATION_DEPTH` 5 · `MAX_FORMATION_MEMBERS` 64 · `MAX_OPERATION_PARTICIPANTS` 128 · `OPERATION_NOTICE_MIN` 24 h / `OPERATION_NOTICE_MAX` 72 h (5A) · `MAX_SEAT_OFFERS_PER_OPERATION` 32 · `SEAT_OFFER_TTL` 7 d · `MAX_CREW_NAMED` 16 · `RECOVERY_COOLDOWN` 14 d.

**Not constants — config.** `[tacsim]`: `enabled`, `pack_paths`, `seed_paths`, `corridor`, `active_domain_limit`, `ground_pulse_ms`, `space_pulse_ms`. `[tacsim.recovery]`: pool size, tiers, cap, thresholds, cooldown, insurance policy (7A). `[tacsim.time]`: the campaign's rate policy and the per-subsystem catch-up policies (8D) — **doc 10 1A's ≈ one OOC hour per IC day is a policy value here, never a constant in code**. Parsed host-side like every other block: `mush-config` → `mush_server::config_ext` → `TacsimRuleset::with_config`, the `[shadowrun.economy]` pattern (08f §9).

---

## 8. The owner's answers (10a, 2026-09-03 — binding)

The owner accepted **every** recommended option — **1A 2D 3A 4A 5D 6A 7A 8D 9A 10D 11A 12A 13A 14A 15A 16A 17A 18A 19A 20A 21A 22A** — plus 10a §4's eleven cuts, §5's handbacks, and the doc 10 R6 amendment (doc 10 is now Draft 0.4).

1. **1A** — deterministic yield curve per deposit with depletion; a survey reveals bands, never the reserve; production is scheduled, journaled lot creation on a deterministic key; variation is discrete campaign events; tenure is a `ClaimRecord`, a licence is an entitlement.
2. **2D** — a neutral inventory authority owns location-bound quantities **and** lots; the ruleset decides representation per commodity; every balance binds to an **E3 custody endpoint**; interconversion is one journaled act each way; a lot has no `legal` and no `custody` field.
3. **3A** — `mush-industry` ships a reservation record **shape-conformant to doc 09 C8** with a named migration **trigger that opens a review, never an automatic migration**; "consume atomically" before K3 means one plan, one batch, one journal-authoritative document, no fallible step after the first install.
4. **4A** — one `mush-propagation` crate owning contacts **and** delivery envelopes over a shared physical context with **separate detection and delivery resolvers**; tactical propagation on the domain tick, coarse scheduled delivery on a campaign cadence otherwise; **an ack is itself a delivery**.
5. **5D** — a new neutral `mush-campaign`; one logical operation with separately versioned frozen `OperationTerms` and mutable `OperationRuntime` under one `OperationId`; the bond is an economy hold; consequences write `ControlRecord` and `Front` through `mush-faction`.
6. **6A** — everything expressible as a table is a versioned TOML content pack in two kinds: immutable **rules-definition** packs referenced by digest, and **campaign-seed** packs instantiating mutable world state once.
7. **7A** — the recovery provider is a **funded organization** running an obligation + lease template; eligibility is a deterministic predicate over **accessible operational assets**; the plan states pool exhaustion, queue priority, cooldown, insurance, replenishment and upgrade terms.
8. **8D** — **one versioned world timeline owned by the kernel** with derived cadence adapters; a rate change opens a new generation and never reinterprets earlier facts; the campaign layer sets rate policy; every subsystem gets an explicit catch-up policy.
9. **9A** — a generic engine `FormationKind` with **composition as a constraint set** (`exact ≡ min == max`), ruleset-authored instances, and strictness scoped to creation / doctrine qualification / an operation's requirement.
10. **10D** — a `SeatOffer`/`SeatGrant` campaign record carrying **the contract's consent-hash discipline by sharing the helper**; an E4 contract only where a fictional principal has terms; the account reference is audit-only; live control is a lease derived from the grant.
11. **11A** — a synchronous account-identity provider in `mush-core` implemented by the server/store layer: `same_account`, `all_accounts_distinct`, atomic focus claim/release, synchronous updates; `None` fails closed; two prerequisites (an immutable id, a boot-hydrated reverse map).
12. **12A** — tactical **ingress** enters through the prepared K lane from the first pulse; **resolution** is one deterministic domain-tick commit per domain per tick, bounded by the K3 bundle limits.
13. **13A** — a neutral `mush-inventory` bound to custody endpoints is **the single source of quantity truth**; industry, markets, contracts and simulation consume it.
14. **14A** — a unique `ClaimRecord` (transferable, pledgeable, contestable, suspendable, seizable) plus separate licence entitlements; overlapping political authority through `ControlKind::{DeJure, DeFacto}`.
15. **15A** — deterministic promotion on four stated triggers; demotion only at a quiet boundary after a durable summary; a hard limit on concurrently active domains with a stated policy at the limit; **state preserved both ways**.
16. **16A** — observations are **local**; a transmitted report creates the *recipient's* record with source, age, confidence and provenance; **unidentified and false contacts from the start**; no omniscient fused record.
17. **17A** — one journaled `operation.resolved` root fact, then **idempotent** consumer transitions; the operation stays `Resolving` until every consumer acknowledges.
18. **18A** — **no hot mutation** of a published definition digest; versions coexist; live units, blueprints, recipes and open contracts stay pinned until an explicit migration or in-world refit.
19. **19A** — an **audit-only** server record maps driver token → account; journals keep only the token; participation history and matchmaking standing consume the audit record and never become IC settlement.
20. **20A** — focus is **per account**, atomic to acquire, with a declared fallback on release; multiple connections may attach to one focus; a clean switch fires the fallback immediately, an unclean disconnect uses the existing linkdead episode; **accepting an invitation does not seize focus**.
21. **21A** — one neutral party/muster subsystem, and it is **E6a's `mush-party`**; there is never a TacSim-specific party record.
22. **22A** — **one coherent narrow vertical slice**: one ground engagement and one local-space interception with authored units; construction balance, the weapon-family matrix, boarding, aerospace transitions and fleet tuning are explicitly later.

---

## 9. Open questions (defaults chosen)

Only what 10a and doc 10 leave open. Each default is what the build takes unless the owner says otherwise.

1. **The active-domain limit.** 15A requires a hard limit and a stated policy but sets neither. Default **`MAX_ACTIVE_DOMAINS` 8**, and at the limit the trigger **queues** — held and re-evaluated each campaign tick, never silently dropped — because "refuse with notice" strands an interception and "degrade to scheduled resolution" is a second combat model.
2. **Tick rates per domain class.** 2A gives bands, not values. Default **ground 2 s, space 6 s**, both config, both inside the bands.
3. **The pack digest algorithm.** Default **SHA-256**, matching `plan_hash` (08c §3.3) and the existing `DefinitionRef` digest discipline, with the domain/encoding version stated in the manifest rather than in the hashed bytes (doc 09 §2.6).
4. **The interim for object-per-unit materials before cut 2.** Default: **there is none.** Cut 1's fixture ships locations, commodity *definitions*, a recipe and a route but **no quantities**, and nothing consumes matter until `mush-inventory` exists. An object-per-unit interim would be a second quantity authority — exactly what 13A exists to prevent — and would need migrating out one cut later.
5. **Which crate owns the C8-conformant reservation for matter.** 3A names `mush-industry`; 13A gives `mush-inventory` "custody reservations". Default: **one shape, two owners** — matter in `mush-inventory`, capacity in `mush-industry` (D-E7-2). Flagged because it reads across two answers rather than following one.
6. **Where the account-identity seam ships.** 10a §4 lists "the Q11 oracle" under both cut 5 and cut 10. Default: **cut 5 ships the seam plus the focus claim/release half** (it cannot enforce one focus without it); **cut 10 adds `same_account`/`all_accounts_distinct`** and their consumers.
7. **How `MAX_UNITS_PER_DOMAIN` is derived.** Default: **measure at cut 8** — encode a worst-case tick for N units and pin the constant where the encoded bundle stays under a stated fraction (60 %) of the K3 caps, with a test that fails when a change pushes past it. 64 is the working figure until the measurement lands.
8. **If E6a has not shipped `mush-party` when cut 10 starts.** Default: **pull E6a's exact cut forward unchanged** (21A's own instruction), never a temporary record.
9. **Where the campaign's restamp-policy registry lives.** Default: **`mush-campaign`** (cut 9), with cut 4 shipping the pause default and the registration seam, so cuts 4–8 run correctly before the campaign layer exists.
10. **Whether `mush-industry` and `mush-inventory` are one crate.** Default: **two.** 13A's neutrality argument is that market, contract deliverables and salvage need quantity truth without an industry dependency; one crate would give them one.
11. **Who ships the scene IC time floor.** Default: **whichever cut lands first owns it**, exactly as `@econ/cadence` was resolved (08f §11 q2); cut 6 ships it if nobody has, because arrival cannot be correct without it.
12. **The ten commodity families' exact ids.** Default: 4A's list transliterated — `metals`, `volatiles`, `carbon_feedstock`, `strategic_elements`, `structural_alloys`, `composites`, `electronics`, `propellant`, `industrial_parts`, `munitions` — authored in cut 1's rules pack and therefore changeable without code.
13. **Whether the K branch merges before cut 5 or E7 develops on it.** Default: **E7 develops on the K branch and merges with it**, because §2.2's grant model is a correctness dependency, not a convenience. Flagged as a sequencing decision the owner may want to make differently.

---

## 10. What E7 hands back

**Fact kinds to register** on E4c's `FactProvider` (`crates/mush-economy/src/facts.rs:62-81`; the table at `crates/mush-server/src/modules.rs:148`, main), each owning a **stable id space**, the economy pointing *at* that journal and never duplicating it (T2). **None exists today** — `mush-jobs` is the only registered provider, so every one of these conditions currently answers `None` (unsatisfied), and `with_facts` panics on a duplicate namespace, so it is one provider per crate:

`sim.journey.arrived` · `sim.journey.failed` · `sim.station.served { actor, station, from, to }` (R6/T13's participation fact, doc 10 §15) · `sim.cargo.delivered` · `sim.docking.completed` · `tactics.sortie.started` · `tactics.sortie.ended` · `tactics.unit.disabled` · `tactics.unit.abandoned` · `tactics.unit.captured` · `tactics.unit.destroyed` · `tactics.salvage.recovered` · `campaign.operation.resolved` (17A).

**Two upstream asks, both from doc 10 §15.** (1) **The scene IC time floor** — `scene_ic_time` is `ic_started_at + (round − 1) × round_seconds` with **no floor or clamp** (`crates/mush-scenes/src/data.rs:171-178`, main), so a journey arriving days later would leave later scene events stamped **before** arrival. Doc 10 6A requires a monotonic floor that `scene_ic_time` respects, raised by journey arrival alongside a typed arrival event; doc 09's `SceneTurn` clock is the natural carrier, and E6b already wants it (08f §7 item 4). §9 q11. (2) **The manual-cadence hook** — `advance_cadence_manual` (`crates/mush-economy/src/clock.rs:297-305`, main) has **no caller at `3967e12`** outside its re-export and one E4a test, and the staff verb the in-flight E5b build adds is not a sim caller either; cut 6 becomes the first at journey boundaries (T1's alignment bands) and cut 8 at sortie boundaries (doc 10 §9).

**What E7 does not hand back, deliberately.** `scene.outcome` stays **E6b's** — composition-root data the server registers under adjudicator/facilitator authority; the canon-outcome-kind table is wired **empty** today (`modules.rs:157`, main), and E7 registers no scene-outcome kind of its own. The market projection stays **E6c's**; E7 supplies **inventory locality** and consumes quotes. The party subsystem stays **E6a's** (21A). **Owner abandonment** (T3; doc 10 §15; 08f §1.1 keeps it "in E7 with the mechanical recipe written out") lands in **cut 3**, beside the claim seizure needing the same unilateral title effect.

---

## 11. Review outcome

*(Placeholder — three passes, as 08c §11 / 08d §12 / 08f §12: an adversarial pass over the state machines and the fog-of-war boundary, a code-grounding pass against both trees, and a design-fit pass against 10 / 10a / 08c / 08d / 08f / 09. Nothing folded yet; this draft has not been reviewed.)*
