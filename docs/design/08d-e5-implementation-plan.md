# E5 implementation plan — World of Darkness housing and background support

**Status:** Draft 0.2 — reviewed (three passes: adversarial, code-grounding, design-fit against docs
08/08a/08c/09 and the owner's answers); every confirmed finding is folded below and itemised in §12.
**E5a is built** — `crates/mush-housing`, the pure library; read §3.5 "As built" for its signatures,
deviations and the seams E5b consumes.
Companion to [08-economy-organizations-ownership.md](08-economy-organizations-ownership.md) (§6.3
abstract wealth, §11.1 the WoD slice, §18-E5), [08a-economy-build-status.md](08a-economy-build-status.md)
(ledger), [08c-e4-implementation-plan.md](08c-e4-implementation-plan.md) (E4, which E5 is layered on),
and the owner's twenty answers of 2026-09-03 recorded verbatim in §10 — **they are the design
constraints and override doc 08 §11.1 where the two differ.** Grounded in the tree at `3a061f6` by
two surveys and the grounding review; every `file:line` was checked. Departures from doc 08 are
numbered decisions (D-E5-n, §8).

> **Gate:** E5a (pure) may be built any time. **E5b starts only after E4b1 ships** (it subscribes to
> E4's cadence clocks, mirrors E4 obligations, and opens E4 escalation records); E5c after E5b, and
> its canon events need E4c's `source`-bearing `CanonRecorded`. Re-ground §4's E4 references against
> the built E4 before E5b starts.

---

## 1. Scope — what E5 is, in the owner's terms

E5 makes a World of Darkness character's background quietly support housing and ordinary life until
a missed obligation becomes a story hook (doc 08 §1), under a **narrow exact/abstract boundary**:

- **Personal monetary capacity is abstract by default.** A character has a *support level* derived
  from Resources and its sources, not a wallet. Routine life posts nothing. A "major purchase" is
  not automatically an exact cash transaction — it may cost strain, need financing, a contract, or a
  scene (owner answer 1).
- **Organization money is exact** (E2/E3 as shipped). **Asset title and tenancy state are exact**.
- **Exact personal money is an opt-in mode** per deployment: `personal_mode = "exact"` gives
  characters a `wod.usd` account and configured starting funds; `"abstract"` never shows them a
  meaningless zero-dollar wallet (answers 2, 3, 15).
- **Housing** is a game-neutral `mush-housing` crate: properties (staff/builder-built ordinary rooms
  or room groups), tenancies, tenant-managed guest access enforced through the movement/carry hooks,
  support assessments, and a story window that **never ends a tenancy automatically** (answers 5–10).
- **Careers** are a `ruleset-wod` layer: one possible source behind Resources among several
  (inheritance, patron, criminal income, domain support), with four deterministic stances that alter
  *pressure and opportunity*, never a second income stream (answers 11–12).
- **Three cross-cutting requirements** the owner added: an **attention budget** (at most one
  unresolved personal economic prompt per character; a bounded per-faction pool of *currently open*
  off-screen operations), **non-response semantics** (ignoring a prompt chooses the conservative
  default or leaves a window pending — never a failed homework assignment), and **front
  attribution** (every economic or faction progress event cites a specific front, opportunity, or
  operation) (18–20).

**Exit criterion (doc 08 §18-E5, adjusted by answer 13):** scenario 23 passes in exact-cash mode,
abstract mode, and `personal_mode = off`, and across large IC-time changes; scenario 24's retainer
payroll is **deferred** with dependents (§1.1) — E5 substitutes "an organization account pays a
character an obligation" (already E4) plus the abstract employer relationship carrying visibility and
risk without posting.

### 1.1 Explicitly out of E5

- **Dependents**: retainers, ghouls, Herd payroll or loyalty/feeding mechanics (answer 13 — Herd is
  not an economic dependent; the Retainers Background has no NPC linkage and a ghoul's `domitor` is
  a free-text field, `ruleset-wod/src/splats/ghoul.rs:204`). `SupportSource::Dependent` is reserved.
- **Routing `home`** (answer 10): `go_home` is a raw teleport (`mush-core/src/commands/movement.rs:139-183`)
  that consults `check_move_attempt` at `:154`, so tenancy fails closed through the hook.
- **Doc 08 §11.1's interim "service/access restrictions" step** during delinquency is deliberately
  **dropped** (D-E5-4, answers 8–9): a delinquent tenant keeps entering their home; pressure is
  strain, status, and the story window, never a locked door.
- **Pushed period summaries**: routine visibility is pull-only (`+career`, `+resources`, `+home`);
  only material changes push (D10, §14.3).
- **A global economy-disable switch** (answer 14).
- **Automatic trait changes** (answer 4): the economy never writes a sheet dot and never files
  trait-loss jobs because a scheduled process ran. Permanent Resources changes go through
  `+xp/spend` (`ruleset-wod/src/xp.rs:112-300`).
- **A `RulesValue` refactor** (answer 16): the K branch has `mush_core::RulesValue`
  (`mechanics.rs`); E5 uses it after the K merge and touches nothing else.
- **Housing reading ruleset sheets** (answer 17).
- **Haven mechanics** (answer 15): Haven dots → a ruleset-side `HavenQualities` note; no mechanics.
- **Markets, quotes, catalogs, fences** (E6/E8); **player `@dig`** through tenancy (answer 5).
- **Changing `check_move_attempt`'s signature** so a hook can see the *issuing* actor of a
  `@teleport`: today it receives the moved object (`building.rs:527`), so a staffer teleporting a
  non-staff stranger into a property is refused like any stranger. Accepted for E5: staff add the
  person as a guest first, or teleport them adjacent. A core change is a later decision.
- **Hook coverage of sim-driven relocation** (route arrival `mush-sim/src/route.rs:211`, transition
  `apply` `transition.rs:482`, domain presentation `commands.rs:331,661`, sim boot `boot.rs:217`) and
  **chargen onboarding placement** (`ruleset-wod/src/onboarding.rs:204,233`): these call
  `World::move_to` directly. Property rooms are ordinary rooms by rule (D-E5-12); a sim presentation
  room or the chargen start room is never a property room, so these paths cannot enter one except by
  builder error, which `@property/rooms` refuses (§4).

---

## 2. Cut structure

| Cut | Crate(s) | One line | Verify axis |
|-----|----------|----------|-------------|
| **E5a** | `mush-housing` (pure) | `Property`/`Tenancy`/`AccessTerms`/`SupportPlan` records, the tenancy state machine (never auto-ends), the multi-tenant support assessment, the access predicate, the story-window and notice-decay rules | fail-open access, state-machine holes |
| **E5b** | `mush-housing` module (+ one `mush-economy` seam consumed) | persistence + boot, the `MoveAttempt`/`CarryAttempt` hooks, **the `Obj.home` sync invariant**, a property `DestroyGate` + `ObjDestroyed` observer, cadence-driven assessment, the exact-mode bridge to E4 obligations, the story window (job + optional scene + decaying re-notify + level-2 escalation), title minting through E4's title-issuance seam, `@property/*`, `@tenancy/*`, `+home`, `+guest` | offline-home safety, notice bounds, hook bypasses |
| **E5c** | `ruleset-wod` (+ host config plumbing in `mushd`) | `[wod.economy]` config, `SpendingProfile`/`CareerProfile`, sources and stances, the period evaluation on the cadence clock, prompts with the attention budget and preemption rule, front attribution incl. orphaned fronts, mode-transition reconciliation, chargen residence step and mode-dependent approval, `+career`, `+prompt` | tedium, budget, attribution, mode isolation |

Dependencies (grounding review: no cycle — org/economy/jobs/faction have no edge back to any
ruleset crate): `mush-housing → {core, org, economy, jobs}`; `ruleset-wod → {housing, economy, org,
jobs, faction, canon}` — faction for `FactionId`/`FrontId` only (`mush-faction/src/data.rs:55`,
`sim.rs:32-43`, re-exported), canon because E5c commits its own evidence events (`mush_canon::commit`).
Today ruleset-wod depends only on `mush-ruleset, mush-dice, mush-core, mush-jobs, mush-llm,
mush-proto`; every edge above is new. **Module order** (08a decision 19): `mush-housing` boots in
`server_modules` after `{org, sim, tactics, economy, jobs}`; `ruleset-wod` boots in `ruleset_modules`
after the base list, so housing is loaded before the first `SupportPlan` is submitted.

E5 consumes one seam E4 must provide (fed back to 08c §6 item 6): **`mush_economy::issue_title(ctx,
object, owner, scope, issuer: SystemPrincipalId)`** for a registered title-issuing system principal
(D-E3-13 — shape only until now; `TransferTitle` moves title between holders, it never mints).
`ruleset-wod` registers `system:wod.housing` as a title issuer at boot.

---

## 3. E5a — `mush-housing`, the pure library

### 3.1 Records

```rust
pub struct Property {
    pub id: PropertyId,
    pub name: SmolStr,
    pub anchor: ObjId,                       // the primary room; the EconomicAsset title record lives on it
    pub rooms: BTreeSet<ObjId>,              // ≤ MAX_ROOMS_PER_PROPERTY (16); PRIVATE ordinary rooms only (D-E5-12)
    pub tier: PropertyTier,                  // 0..=5: the residence's physical class and upkeep, NOT haven quality
    pub landlord: PrincipalId,               // Organization(..) (a named landlord org) or System(wod.housing)
    pub landlord_scope: Option<OrgScopeId>,  // the org compartment that manages it (D-E3-5)
    pub upkeep: Option<Amount>,              // exact-mode rent per cadence period; None in abstract mode
    pub features: Vec<Feature { key: SmolStr, approved_by: ObjId }>,   // ≤ MAX_FEATURES (16)
    pub status: PropertyStatus,              // Vacant | Let | Withdrawn
    pub revision: u64,
}
pub struct Tenancy {
    pub id: TenancyId,
    pub property: PropertyId,
    pub tenants: BTreeSet<ObjId>,            // ≤ MAX_TENANTS (8)
    pub guests: BTreeSet<ObjId>,             // tenant-managed, ≤ MAX_GUESTS (16)
    pub terms: AccessTerms { inspection: Option<InspectionTerm { who: PrincipalId, when: Always | WhileDelinquent }> },   // explicit only (answer 9)
    pub funding: Funding,                    // Abstract { plans: BTreeMap<ObjId, SupportPlan> } | Exact { contract: ContractId, obligation: ObligationId }
    pub cadence: CadenceId,                  // the campaign clock this tenancy is assessed on (answer 7)
    pub homed: BTreeSet<ObjId>,              // tenants whose core `Obj.home` housing set into this property (§3.3 invariant)
    pub status: TenancyStatus,               // Pending | Active | Grace { since } | Delinquent { since } | EvictionPending { window: StoryWindow } | Ended { how: Resolution }
    pub history: Vec<TenancyEvent>,          // bounded (32)
    pub revision: u64,
}
pub struct StoryWindow { pub opened: i64, pub job: Option<u64>, pub scene: Option<SceneId>, pub notified: u8, pub escalation: Option<EscalationId>, pub last_notice: i64 }
pub enum Resolution { TenantLeft, StaffEnded { by, reason }, Waived { by }, Settled }
/// What the ruleset asserts about ONE resident's means, per cadence period.
pub struct SupportPlan {
    pub resident: ObjId,
    pub level: SupportLevel,                 // 0..=5 — derived by the ruleset, opaque to housing
    pub sources: Vec<SupportSource>,         // Career{..} | Patron | Inheritance | Criminal | Domain | Dependent(reserved) — display/attribution only
    pub asserted_at: i64,
    pub revision: u64,
}
```

Only the housing module's own `submit_plan` entry (called by the ruleset's evaluator, never by a
player command) writes `plans`; a plan is keyed by its resident and must name a current tenant.

### 3.2 The state machine (D-E5-1: a home never disappears on its own)

```text
Pending ── landlord + tenant accept ──▶ Active                      (Obj.home set for each tenant, §3.3)
Active ── period Unsupported / rent missed ──▶ Grace{since}         (one private notice)
Grace ── supported again / paid ──▶ Active                          (silent)
Grace ── GRACE_PERIODS elapsed ──▶ Delinquent{since}                (one private notice; landlord notice)
Delinquent ── supported again / arrears paid ──▶ Active             (silent)
Delinquent ── DELINQUENT_PERIODS elapsed ──▶ EvictionPending{window} (one private notice; job opened; optional scene)
EvictionPending ── supported again / paid ──▶ Active                (the window closes; job resolved "recovered")
EvictionPending ── explicit resolution ──▶ Ended{how}               (tenant `+home/leave`, staff `@tenancy/resolve end|waive`, contract settlement)
EvictionPending ── window elapses ──▶ EvictionPending               (decaying re-notify; level-2 escalation; NO state change)
```

**Only an explicit act ends a tenancy** and clears the home anchor / revokes access: a tenant's own
move-out, a staff resolution after a scene, or a deliberate staff waiver. The story window is
*bounded* in notices, not in duration — an offline player's home persists indefinitely (answer 8;
decision 25's ladder handles the paging). **Notice decay** (design-fit E): the tenant is re-notified
at most `MAX_WINDOW_NOTICES` (3) times at `WINDOW_RENOTIFY`, then **nothing further until the level-2
escalation opens** (at `WINDOW_ESCALATE`), then at most weekly — never daily forever.

### 3.3 Assessment, access, and the home invariant (pure)

```rust
/// Abstract funding: the BEST asserted support among the tenancy's residents vs the property's tier
/// (any one paying tenant covers a shared lease; a co-tenant cannot drag the assessment down).
pub fn assess(tier: PropertyTier, plans: &BTreeMap<ObjId, SupportPlan>) -> Assessment;   // Supported | Strained | Unsupported | Unknown (no plan → Unsupported only after UNKNOWN_GRACE periods)
/// The hook predicate: may `who` enter (or take from) a room of `property` under `tenancy`?
pub fn may_enter(t: &Tenancy, terms: &AccessTerms, who: ObjId, is_staff: bool, now: i64) -> Access;   // Tenant | Guest | Inspection | Staff | Refused
```

Rules: tenants and guests always (regardless of status — a delinquent tenant still enters their
home); `Inspection` only under an explicit term and, for `WhileDelinquent`, only in
`Delinquent`/`EvictionPending` — **legal authority, economic title, and physical access remain
separate** (answer 9); staff bypass applies to the *moved object* being staff; everyone else refused
with one neutral text. Carry mirrors enter: an actor who may not enter may not take from inside.

**The `Obj.home` invariant (D-E5-11).** Core's destroy dispersal moves a destroyed room's contents
to each object's `Obj.home` through raw `move_to` with no hook (`mush-core/src/world.rs:885-899`), and
`go_home`'s routing only helps if `Obj.home` points at the tenancy. So housing keeps the two in
lockstep: on `Active`, `World::set_home(tenant, anchor)` for every tenant (recorded in `homed`); on
`Ended`, `set_home(t, None)` for every object in `homed` **and** for every guest or item whose
`Obj.home` resolves inside the property's rooms. An evicted tenant caught in a destroyed room
therefore disperses to Limbo, never back into the property.

### 3.4 E5a tests (planned ~30; **38 as built**, **55 after the verify fold** — 54 in `crates/mush-housing/tests/e5a.rs`, 1 in-crate)

Every transition above and every forbidden one (no path to `Ended` without a `Resolution`); a
window elapsing changes nothing but the notify count and stops after `MAX_WINDOW_NOTICES`; recovery
from each failure state; `assess` across tier/level combinations, best-of with two residents, a
resident whose plan is missing, `Unknown`; `may_enter` for tenant, guest, inspection-always,
inspection-while-delinquent (refused when Active), staff, stranger; the `homed` set computed for
activation and cleared for ending; caps. The fold adds 17 named `vf_<lens><n>_…` regression tests,
one per confirmed verify finding (§3.5, §3.6).

### 3.5 As built (E5a, `crates/mush-housing`)

Built as a pure workspace crate depending on `mush-core`, `mush-org`, `mush-economy`, serde,
smol_str and thiserror — **no** ruleset crate, **no** `mush-scenes`, **no** `mush-jobs` (a story
job is a bare `u64`). `cargo test -p mush-housing` 38 passing; `cargo clippy --workspace
--all-targets -- -D warnings` clean; `cargo test --workspace` **2591 passing, 0 failing**. `cargo
fmt` stays off, unchanged. **After the verify fold** (below): `cargo test -p mush-housing` **55
passing** (54 + 1 in-crate); workspace clippy clean; `cargo test --workspace` **2608 passing, 0
failing**.

**Files.** `src/ids.rs` (`PropertyId`, `TenancyId`, `SceneRef`) · `src/record.rs` (every §3.1
record + the §9 caps) · `src/error.rs` (`HousingError`) · `src/tenancy.rs` (the state machine + the
window) · `src/assess.rs` (`assess`/`harden`) · `src/access.rs` (`may_enter`/`may_take`) ·
`src/home.rs` (the D-E5-11 helpers) · `tests/e5a.rs`.

**The state machine as built** (`src/tenancy.rs`), one pure `fn` per arrow, each returning
`Result<Tenancy, HousingError>`:

| from | fn | to | guard / refusal |
|---|---|---|---|
| `Pending` | `accept(t, landlord_accepted, tenant_accepted, now)` | `Active` (+ `homed`) | landlord **and** every tenant in `tenant_accepted`, else `NotAccepted`; off-`Pending` → `State`; ended → `AlreadyEnded` |
| `Active` | `period_missed(t, now)` | `Grace{since: now}` | `Pending` → `State`; already in the ladder → unchanged (the run is never restamped) |
| `Grace`/`Delinquent`/`EvictionPending` | `period_supported(t, now)` | `Active` | `Pending` → `State`; already `Active` → unchanged, no revision bump |
| `Grace{since}` | `grace_elapsed(t, now)` | `Delinquent{since: now}` | `now - since ≥ GRACE_PERIODS`, else `NotDue`; off-`Grace` → `State` |
| `Delinquent{since}` | `delinquent_elapsed(t, now)` | `EvictionPending{window opened at now}` | `now - since ≥ DELINQUENT_PERIODS`, else `NotDue`; off-`Delinquent` → `State` |
| `EvictionPending` | `attach_window(t, job, scene)` | `EvictionPending` | records the job/scene E5b opened; no window → `NoWindow` |
| `EvictionPending` | `window_elapsed(t, now, escalation)` | `EvictionPending` (**never** a state change) | `window_due` decides: `Notify` while `notified < MAX_WINDOW_NOTICES` at `WINDOW_RENOTIFY`; then `Silent`; then `Escalate` at `WINDOW_ESCALATE` past the last notice (a `None` escalation id is `NeedsEscalation`, never a skipped page); then `NotifyEscalated` every `WINDOW_ESCALATED_RENOTIFY` |
| any non-`Ended` | `resolve(t, how, now)` | `Ended{how}` | the **only** producer of `Ended`; ended → `AlreadyEnded` |
| `Active`…`EvictionPending` | `advance(t, assessment, now)` | composed of the above | `Unknown` moves nothing; `Pending` → `State` |

`Ended` is unreachable without a `Resolution` **by type** (the variant carries one) and by
construction (`resolve` is its only producer); `tests/e5a.rs` pins it two ways — the automatic half
driven 40 periods never ends anything, and the fuzz re-checks it after every step.

**Deviations and clarifications** (all recorded, none silent):

1. **`WINDOW_ESCALATE` is measured from the last notice, not from window open.** Read literally
   against `MAX_WINDOW_NOTICES` 3 at `WINDOW_RENOTIFY` 1, §9's "2 periods" would fire the escalation
   at period 2 while the third re-notice is still due at period 3, making §3.2's "then **nothing
   further** until the level-2 escalation opens" describe a gap that cannot exist. As built, a
   window opened at `p` re-notifies at `p+1, p+2, p+3` and escalates at `p+5`.
2. **"then ≤ weekly" is `WINDOW_ESCALATED_RENOTIFY = 7` periods**, because the library reads no
   wall clock. Seven periods is ≥ a week on any cadence of a day or longer, which is every cadence
   the three-period ladder is meaningful on; E5b re-checks it when it registers the clock.
3. **`Unknown`'s grace needs a counter the record does not carry.** `assess(tier, plans)` keeps its
   §3.3 signature and answers `Unknown` when no resident has a plan; `harden(a, unknown_periods)`
   turns it into `Unsupported` once `unknown_periods > UNKNOWN_GRACE`. E5b keeps the per-tenancy
   consecutive-`Unknown` count — it is per-period module state, so the pure library did not invent a
   record field for it.
4. **`Assessment::Strained` counts as covered.** The band rule is `level ≥ tier` → `Supported`,
   `level + 1 == tier` → `Strained`, `level + 2 ≤ tier` → `Unsupported`, no plans → `Unknown`. A
   strained tenancy does not slip: the rent is being paid, and §5.2's strain is the ruleset's cost.
5. **`Pending` and `Ended` tenancies admit nobody but staff.** §3.3's "tenants and guests always
   regardless of status" is about the *failure ladder* (`Delinquent` still walks in); a tenancy
   nobody has agreed to, and one whose ending "revokes access", grant nothing. Fail closed.
6. **`WhileDelinquent` also requires `now ≥ since`.** A `since` stamp in the future (a rewound or
   re-pointed cadence, a restored backup) refuses rather than reading as "delinquent already": a
   clock error must not open a door.
7. **`to_unhome_on_end` narrows §3.3's "every object in `homed`"** by one safety carve-out: an id
   whose `Obj.home` now resolves *outside* the property's rooms is left alone, because another
   tenancy owns that pointer and clearing it would break *that* home's invariant. Everything else in
   `homed` (including a home that resolves to nothing) is cleared, plus every tenant or guest homed
   inside.
8. **`SceneRef` is an opaque `u64`, not `mush_core::SceneId`.** Core scene ids restart at 0 every
   boot, so a persisted `SceneId` would name a *different* scene after a restart.
9. **String caps beyond §9**, in the `mush-economy` "every stored string is bounded" discipline:
   `MAX_PROPERTY_NAME_BYTES` 64, `MAX_FEATURE_KEY_BYTES` 32, `MAX_REASON_BYTES` 256,
   `MAX_PLAN_SOURCES` 8. `SupportSource` is **not** `Ord` (`mush_org::PrincipalId` is deliberately
   unordered).
10. **No state families, no authoritative store lane.** E5a registers nothing with K3's
    `action-state/*` families (`mush-core/src/{action_state.rs,authoritative.rs}`, doc 09 Status
    0.11) — 08c §12's "K3 review" keeps E-series records conventional. **Handoff:** E5b should
    revisit this for `tenancy:{id}`, whose status ladder is exactly the shape a typed state family
    models, before it writes its persistence.

**Seams E5b consumes** (beyond the §3.1/§3.3 surface):

- `may_enter_with(t, terms, who, inspector, is_staff, now)` / `may_take_with(..)` — `may_enter`
  resolves only a `PrincipalId::Object` inspection term and **refuses** an org or system one; E5b
  answers "does `who` hold that principal's authority" through `mush_org::authorize_in_world` and
  calls the `_with` form. `may_take` *is* `may_enter`, so the two predicates cannot drift.
- `window_due(t, now) -> WindowAction { Silent | Notify | Escalate | NotifyEscalated }` — ask before
  acting; mint an `EscalationId` only for `Escalate`, then fold it through `window_elapsed`.
- `homed_inside(candidates, resolve_home, property_rooms)` — the content sweep at ending time
  (`to_unhome_on_end` covers the tenancy's own roster; items need a world walk only E5b can do).
- `Tenancy::remove_tenant` refuses the **last** tenant: that would end a tenancy nobody resolved.
- `periods_since(since, now)` is the crate's one period-arithmetic helper: `0` for a clock that
  moved backwards, `HousingError::Overflow` at the `i64` extremes, never a saturated "a very long
  time" that fires every elapse arrow at once.

**Verify-fold deltas** (the end-of-build adversarial verify's confirmed findings, folded; one
regression test each in `tests/e5a.rs`'s `---- E5a verify fold ----` section, named `vf_<lens><n>`):

| # | as built | as folded |
|---|---|---|
| A1 | `remove_tenant` dropped the leaver from `homed` | `homed` is **not** the roster — it is the list of `Obj.home` pointers housing set, and only the ending sweep clears them; dropping the id left a live home into a property they no longer rent (with A2's anchor rule this closes D-E5-11 on both sides) |
| A2 | `Property` derived `Deserialize` | `#[serde(try_from = "PropertyWire")]` through `Property::new` (anchor ∈ `rooms`, caps, no duplicate feature keys, non-empty name) — the `mush-core` `DefinitionRef` / `mush-tactics` `TacticalUnit` precedent; **and** `to_unhome_on_end`/`homed_inside` now take the `anchor` and treat it as inside the property whether or not the caller's `property_rooms` still lists it |
| A3 | `Tenancy` derived `Deserialize` | `#[serde(try_from = "TenancyWire")]` through `Tenancy::new`, plus the guest cap, tenant ∉ guests, the history cap, and "a plan is filed under its own resident" (the key *is* D-E5-13) |
| A4 | the `WhileDelinquent` comment described only the rewound-clock half | "A stamp in the future refuses; the period the stamp names itself admits." |
| A5 | — | doc note on `to_unhome_on_end` + the E5b handoff below (re-home after unhome) |
| A6 | `InspectionWhen: Default = Always` | no `Default` at all — anything reaching for one minted a standing right of entry nobody wrote down; a term that does not say *when* now fails to load |
| A7 | `Access: Ord`, ranked `Tenant` … `Refused` | `PartialOrd`/`Ord` dropped (the derive made `min()` return the *most* permissive answer) and replaced by `Access::strictest(a, b)`, `Refused` < `Inspection` < `Guest` < `Tenant` < `Staff` |
| S1 | `remove_tenant` mutated an `Ended` tenancy | `AlreadyEnded`, as `add_guest` is |
| S2 | `window_due` answered `Silent` on `Err(Overflow)` — permanently, since nothing else moves `last_notice` | fails **toward paging**: `Escalate` when no escalation is recorded yet, `NotifyEscalated` when one is; never silence, and never a second escalation over the first |
| S3 | every restated plan bumped a revision and pushed a `PlanSubmitted` — 32 of them are the whole tail | a restatement in the same period is a **pure no-op**; in a new period it refreshes the freshness stamp with a revision and no event; only a changed `level`/`sources` is history |
| S4 | a plan never expired, so a dead evaluator carried the lease forever and `UNKNOWN_GRACE` could never fire | `SupportPlan::asserted_period` (`#[serde(default)]`) + `assess_at(tier, plans, tenants, current_period, max_age)`: a plan older than `PLAN_MAX_AGE_PERIODS`, or belonging to a non-tenant, is **absent** → `Unknown` → `harden`. `assess` stays as the ageless, roster-blind form, documented as such |
| S5 | `remove_room` allowed while `Let` | refused: a room dropped from a let property is one somebody lives in that the hook stops answering for |
| S6 | `attach_window` bumped a revision for a no-op and silently replaced an existing job/scene | unchanged clone when nothing changes; a *different* id is `WindowAlreadyAttached` (same guard on the escalation id in `window_elapsed`) |
| S7 | every post-escalation re-notice pushed a `Notified` entry whose count never changed | no history entry — with S3 the 32-deep tail keeps `Accepted`, every `StatusChanged`, `Escalated` and `Ended` |
| S8 | recovery discarded the window and its ids | `Tenancy::last_window: Option<StoryWindow>` (`#[serde(default)]`, stamped `closed`) records the outgoing window on `EvictionPending → Active`, so E5b can resolve the job "recovered" and ack the escalation |
| S9 | the notice budget re-armed on every relapse, so a flickering tenancy was re-notified forever and never escalated | hysteresis: a window reopening within `RELAPSE_WINDOW_PERIODS` of the last one closing (allowing for the ladder's own `GRACE_PERIODS + DELINQUENT_PERIODS`) inherits its `notified` and `escalation` |
| S10 | `best_level` returned the stored newtype, which a hand-written document can load with 255 in it | `Some(SupportLevel::new(max))` |
| S11 | — | E5b's boot reconciliation must drop a destroyed tenant through `remove_tenant` (which takes the plan with them), never by editing the sets; `assess_at`'s roster check is the library-side guard |

Two constants join §9: **`PLAN_MAX_AGE_PERIODS` 2** and **`RELAPSE_WINDOW_PERIODS` 2**.

Three fold deviations, recorded rather than silent:

11. **`assess_at` takes the roster** — the verify wrote it as `assess_at(tier, plans, current_period,
    max_age)`, but "ignore a non-tenant's plan" cannot be answered from a plan map alone, and passing
    the `Tenancy` instead would let an *exact* tenancy be assessed on plans it does not have. The
    signature is `assess_at(tier, plans, tenants, current_period, max_age)`.
12. **The relapse gap is measured on top of the ladder.** `RELAPSE_WINDOW_PERIODS` 2 read literally
    against "the new window opens within 2 periods of the last one closing" can never fire — the
    fastest possible relapse is `GRACE_PERIODS + DELINQUENT_PERIODS` (3) after recovery. So the
    comparison is `closed → now ≤ GRACE_PERIODS + DELINQUENT_PERIODS + RELAPSE_WINDOW_PERIODS`, i.e.
    two periods of slack on the soonest a window *could* reopen.
13. **`SupportPlan::new` takes the period** as a fifth argument (`asserted_at` is the raw cadence
    position `@tenancy/show` prints; `asserted_period` is the monotonic ordinal freshness is measured
    in, so staleness survives a re-pointed clock). `StoryWindow` gains `closed: Option<i64>`, set only
    on the copy parked in `last_window`.

**Handoffs to E5b** (with A8's, which this fold does not implement):

- **One live tenancy per property**, enforced by the module and picked *explicitly* — never "any live
  tenancy on this room admits".
- **`add_guest` subjects must be characters.** The library accepts any `ObjId`.
- **Re-home after unhome** (A5): `to_unhome_on_end` clears per object, and an object can be a tenant
  of a second live tenancy; after applying the set, re-home anybody still on another live lease.
- **Use `remove_tenant` at boot** (S11), never a direct edit of `tenants`/`homed`/`funding`.
- **Resolve `last_window`'s job and ack its escalation on recovery** (S8), and on a resolution read
  the window off the tenancy passed in.
- **`assess_at`, not `assess`,** on the cadence tick, with the module's own period counter.

### 3.6 E5a verify outcome (2026-09-03)

Two adversarial lenses over the shipped cut. **Lens A (access, the home invariant):** eight findings,
all confirmed by probe — two MEDIUM (`remove_tenant` forgetting the home anchor; both record types
deserializing straight past their own constructors, which made every §9 cap advisory on load), five
LOW (a comment contradicting the pinned `WhileDelinquent` behaviour; `InspectionWhen` defaulting to
the *wider* term; `Access`'s derived `Ord` ranking `Refused` last, so the obvious strict combinator
failed open; the two-live-tenancies gap; the anchor treated as merely one room among many), one
doc-only handoff. **Lens S (state machine, time, boundedness):** eleven findings — three MEDIUM (an
`Ended` tenancy still losing tenants; `window_due` turning one arithmetic overflow into permanent
silence on an open eviction; plans that were both noisy and immortal, which also made `UNKNOWN_GRACE`
unreachable), seven LOW, one doc/guard. No finding touched D-E5-1: nothing in either lens found a
path to `Ended` without a `Resolution`, and the fuzz and the 40-period drive still pin it.

The two that mattered most are the ones where a *quiet* failure replaced a loud one: S2 (a corrupt
stamp silencing an eviction forever) and S9 (a relapsing tenancy notified forever and escalated
never). Both now fail toward paging. The rest are the same shape as E3/E4's folds — persisted
records must re-enter through their constructors, and a bounded audit tail must not be spent on
events that say nothing.

**E5b takes forward:** one live tenancy per property, picked explicitly; `add_guest` subjects must be
characters; re-home after unhome (A5); `remove_tenant` at boot (S11); resolve `last_window`'s job and
escalation on recovery (S8).

---

## 4. E5b — the housing module

- **Persistence:** `property:{id}`, `tenancy:{id}` container docs (corrupt → fail boot); boot
  reconciles: anchor/rooms missing or `GOING` → property `Withdrawn` + a level-2 escalation (a home
  whose rooms vanished is a staff question, never an auto-end); a tenant object missing → dropped
  through `Tenancy::remove_tenant` (roster, plan and a history event — **not** `homed`, which the
  ending sweep owns; §3.5 A1/S11), never by editing the sets; an exact tenancy whose obligation is
  missing → flagged and escalated, not `EvictionPending`. Allocators + meta high-water (the E3 fold rule).
  Conventional docs, per 08c §12's default for E4's own records (state families at K3).
- **Title:** creating a property mints the anchor's `EconomicAsset` title to the landlord principal
  through `mush_economy::issue_title` with issuer `system:wod.housing` (§2). `Obj.owner` of the
  rooms stays the builder — never the org (`@chown` requires a player, `building.rs:631-657`; §9.1).
- **Room rule (D-E5-12):** `@property/rooms` accepts only ordinary `Room` objects that are not sim
  spaces, not domain presentation rooms, not the chargen start room, and not already in another
  property; a room with an exit from a non-property room other than the property's declared
  entrance is refused as a thoroughfare. Common areas are simply not property rooms.
- **Access enforcement:** an `EventHook` on `HookPoint::MoveAttempt` / `CarryAttempt`
  (`mush-core/src/module.rs:305-326`, fired from `movement.rs:98,154`, `building.rs:149,527`,
  `items.rs:55,104,160`; first veto wins, `module.rs:1617-1643`) — the hook answers only for rooms of
  a `Let` property.
- **Destroy gate + observer:** a housing `DestroyGate` (`reg.destroy_gate`, the economy's
  `EconomicDestroyGate` precedent) refuses `@destroy` of any room of a `Let` property ("withdraw the
  property first"); the `ObjDestroyed` observer marks a property whose room died `Withdrawn`, keeps
  every tenancy on it as-is, and opens a level-2 escalation — doc 08 §15.3's "a property with active
  tenancy" gate.
- **Assessment tick:** housing subscribes to the E4 **cadence clock** named by each tenancy: when
  the clock's period advances, assess every abstract tenancy on it (bounded per fire,
  `MAX_ASSESSMENTS_PER_FIRE`; remainder next fire), applying §3.2 with **one private notice per
  transition** and none on `Supported`. Exact tenancies mirror their E4 obligation's
  `ObligationChanged` events (`Grace`/`Delinquent` from the obligation; `EvictionPending` after
  `DELINQUENT_PERIODS` on the same cadence).
- **Story window:** entering `EvictionPending` opens a job in queue `wod-story` (registered with
  `Registrar::job_queue`, `module.rs:1500`; staff/ST-visible under mush-jobs' per-queue visibility;
  `OpenJob.opener` is explicit, so a tick's `GOD` actor is not an issue — `mush-jobs/src/api.rs:94,113`)
  titled for the ST with the tenancy summary and the recommended commands (`@tenancy/resolve
  <id>=extend|end|waive`), optionally starts a scene (`Ctx::start_scene`, `module.rs:1964-1969`) when
  the ST asks via the job; re-notifies per §3.2's decay; after `WINDOW_ESCALATE` opens a level-2
  escalation (08c §4.6) carrying the same commands. Nothing else happens.
- **Notices for offline players:** a `Connected` hook renders pending tenancy notices on reconnect
  (the `mush-mail` pattern, `mush-mail/src/lib.rs:156-172`; `mush_mail::deliver` is crate-private,
  so housing keeps its own bounded pending-notice list per tenant).
- **Existence hiding:** `mush_economy::NamedResource` is reusable; the renderer helpers are private
  (`commands.rs:351,378`), so housing reimplements the same discipline: a property or tenancy the
  actor may not view is "No such property." / "No such tenancy." for nonexistent and unviewable
  alike. View = tenant, guest (reduced), landlord-scope `property.view`, or staff.
- **Commands:** staff `@property/create <name>=<anchor room>[,rooms…] tier=<n> landlord=<org|housing> [upkeep=]`,
  `@property/rooms|tier|feature|withdraw`, `@tenancy/create <property>=<tenant…> [cadence=] [inspect=<who>:<always|delinquent>]`,
  `@tenancy/resolve <id>=extend|end|waive`, `@tenancy/show|list`; **landlord officers** (holders of
  `property.view` / `property.manage` on the landlord scope — new vocabulary entries) may
  `@property/show|list` and `@tenancy/show` within their compartment; players `+home` (a tenant
  sees residence, status, guests, the pending notice; a **guest** sees only the property name and
  their own guest standing), `+home/leave` (explicit move-out → `Ended{TenantLeft}`), `+guest
  add|remove <who>`.

Tests (~30): scenario 23's housing half in abstract mode — supported periods are silent; an
`Unsupported` plan walks Grace → Delinquent → EvictionPending with exactly one notice each and no
movement; the window elapsing re-notifies three times, then is silent until the escalation, and the
tenant still enters and `home` still works; `@tenancy/resolve end` clears `Obj.home` for every
`homed` tenant and the hook then refuses; **an evicted tenant standing in a room that is then
destroyed lands in Limbo, not the property**; support restored at any stage returns to Active
silently; two co-tenants with different plans assess best-of; a stranger, a guest, an
inspection-term holder, and a staffer against each status; `@teleport` of a non-guest by staff is
refused and of a guest allowed; `@property/rooms` refuses a thoroughfare, a sim space, and a room
already owned; `@destroy` of a Let room refused; a room destroyed programmatically → Withdrawn +
escalation, tenancy intact; the offline tenant's reconnect notice; a landlord officer sees their
own properties and nothing of another landlord's; a guest's `+home` shows no tenant list; exact
mode via a real E4 obligation with the same transitions; reboot at every state; large cadence jumps
assess once with bounded catch-up.

---

## 5. E5c — the WoD layer

### 5.1 Config (answer 14)

```toml
[wod.economy]
personal_mode = "abstract"      # off | abstract | exact
housing = true
careers = true
default_engagement = "ambient"  # ambient | engaged
cadence = "chronicle.period"    # the economy cadence clock careers and abstract housing are assessed on
```

Parsed **host-side** (`mushd`, like `WodConfig`/`OnboardingConfig` today — ruleset-wod has no
`mush-config` dependency) into a `WodEconomyConfig` passed into `WodRuleset`. `off` creates no WoD
economic profile; the platform economy is untouched. In `abstract` no personal `wod.usd` account
exists or is shown. In `exact` ruleset-wod registers `wod.usd` (scale 2), the faucet/sink principals
`system:wod.income`, `system:wod.housing` (also the title issuer), `system:wod.chargen`, and the
Resources-to-income table (`[wod.economy.income]`, one amount per dot) is authoritative **only** in
this mode (answer 3). Each income line is a **faucet-funded obligation** created through
`mush_economy::create_system_obligation` with `system:wod.income` as the caller's faucet — never
`@obligation/create`, which refuses a `System` debit outright — and every per-period amount in the
table is bounded by `[economy].mint_obligation_cap_major` (per-instrument override
`[economy.instruments."wod.usd"]`), re-checked at every settlement (08c §4.8, M1).

**Mode transitions on a live deployment** (adversarial F9): flipping `personal_mode` is a staff act
followed by `@wod/economy migrate`, a one-shot reconciliation that lists what changed and what it
did: abstract → exact opens a zero-balance `wod.usd` account per profiled character (staff may seed
with an attributed `@econ/mint`; nothing is minted automatically); exact → abstract creates a
`SpendingProfile` per character from Resources and **hides** `wod.usd` from `+resources` — accounts
and balances persist untouched (they are economy infrastructure); `housing = false` leaves existing
tenancies intact and stops new ones; `off` never deletes a profile, it stops evaluating it.

### 5.2 Profiles, sources, stances

```rust
pub struct SpendingProfile {                 // ruleset-wod's, per character (`wod:profile:{obj}`)
    pub resources: u8,                       // read from backgrounds:Resources at evaluation (SheetDoc::dots, mush-ruleset/src/doc.rs:121), cached for display
    pub sources: Vec<SupportSource>,         // Career(CareerProfile) | Patron{who} | Inheritance | Criminal | Domain{org} | Dependent(reserved)
    pub level: SupportLevel,                 // derived: resources ± stance modifiers, clamped 0..=5
    pub strain: u8,                          // 0..=STRAIN_MAX (10); decays 1/period when Supported
    pub engagement: Engagement,              // Ambient | Engaged (player-set, default from config)
    pub haven: Option<HavenQualities>,       // Haven dots interpreted as qualities — recorded, no mechanics
    pub prompt: Option<Prompt>,              // THE one unresolved prompt (answer 18)
    pub revision: u64,
}
pub struct CareerProfile { pub kind: SmolStr, pub employer: Option<PrincipalId>, pub stance: Stance, pub time_demand: u8, pub visibility: u8, pub risk: u8, pub front: Option<FrontRef> }
pub enum Stance { Coast, Advance, Exploit, Conceal }
pub struct FrontRef { pub faction: FactionId, pub front: FrontId, pub operation: Option<u64> }   // answer 20
```

Stance effects (answer 11), all deterministic per period:

| Stance | support level | strain | opportunities | exposure |
|---|---|---|---|---|
| `Coast` | as derived | −1 decay | rarely (1 per `COAST_OPPORTUNITY_PERIODS`) | none |
| `Advance` | as derived | +1 per period (time demand) | yes — advancement prompts; **no automatic dot gain** | low |
| `Exploit` | **+1 temporary** (this period) | +1 | yes | **high** — a canon evidence event per period, front-attributed |
| `Conceal` | −1 | 0 | none | reduced visibility (no evidence events) |

The period evaluation runs on the cadence clock for every character with a profile: recompute
`level`, apply strain delta, generate at most one opportunity (subject to the attention budget and
the per-faction pool), emit the front-attributed evidence for `Exploit` (`mush_canon::commit`), and
**submit a `SupportPlan` to `mush-housing`** for each tenancy the character holds (answer 17). In
`exact` mode the evaluation additionally maintains the income obligation (faucet or employer account
→ personal account); in `abstract` nothing posts (answers 3, 12). Stance changes take effect at the
**next** period boundary, never the current one (a flip to `Exploit` seconds before assessment
changes nothing).

### 5.3 Attention budget, non-response, attribution (answers 18–20)

- **One unresolved prompt per character.** `Prompt { id, kind: Opportunity{front} | HousingPressure
  | StrainRelief | Advancement, options, default: StatusQuo | PendingWindow, opened_at,
  expires_after_periods }`. **Preemption (D-E5-8):** a `HousingPressure` prompt replaces an open
  `Opportunity`/`Advancement` (the displaced one is dropped with a history note); an `Opportunity`
  never displaces a `HousingPressure`; otherwise a second prompt is dropped, not queued. Housing's
  own notices are notices, not prompts.
- **Per-faction off-screen pool.** Counts opportunities that are **currently open** (prompt not yet
  resolved or expired) and attributed to a faction's front, across all characters, against
  `MAX_OFFSCREEN_OPS_PER_FACTION` (8); resolution or expiry decrements it. When full, no new
  front-attributed opportunity is generated for that faction. E5 counts its own; the pool is the
  reserved seam for the faction simulation to share.
- **Non-response.** When a prompt expires, its `default` applies: `StatusQuo` (nothing changes; a
  history note) or `PendingWindow` (the story window stays open for the ST). Expiry never adds
  strain, lowers support, moves money, or notifies as a failure.
- **Front attribution.** Every opportunity, every `Exploit` evidence event, and every housing story
  job carries a `FrontRef` when it concerns a faction; canon events use `kind = "wod.career.exposure"
  | "wod.housing.pressure"` with `source: "front:{faction}:{front}"` and the operation id in
  `detail`. A generic `scene.concluded` with any faction member is **never** an attribution (08c §6
  item 2 matches on `source`). **Orphaned fronts** (a `FrontId` that no longer resolves after a K or
  faction change): renders as the raw id marked "(orphaned front)", never fails a read, and stops
  generating new opportunities; a test simulates a front's removal.

### 5.4 Chargen and approval (answer 15)

Chargen already runs through a job with its own payload handler (`chargen_cmds::CHARGEN_HANDLER`,
queue `wod-chargen`, registered at `ruleset-wod/src/lib.rs:374-381`; the XP handler at `:386-391` is
the separate precedent). That handler creates, per `personal_mode`: nothing (`off`); a
`SpendingProfile` from Resources (`abstract`); a personal `wod.usd` account plus configured starting
funds minted from `system:wod.chargen` (`exact`). A new chargen stage **Residence** lists vacant
staff-built properties whose tier the derived support level can carry — names and tiers only, never
who lives elsewhere — and lets the player choose one (or none); approval creates the tenancy
(`Pending → Active`) through housing. Haven dots never select or discount a property; they populate
`HavenQualities`.

### 5.5 Commands

`+career` (view; `/stance <coast|advance|exploit|conceal>`; `/engage on|off`; `/source add|remove …`),
`+prompt` (view the one pending prompt; `+prompt <option>`), `+resources` (abstract: support level,
strain, sources; exact: balance via the economy's `@account/show` rules). Staff: `@wod/economy`
(show effective config; `migrate`), `@career/set <who>=…` (employer, front).

Tests (~32): scenario 23 end to end in abstract mode (Resources 3 quietly supports a tier-2
residence for N periods with no output; drop Resources via a staff `@sheet/set` → the next period's
plan is `Unsupported` → Grace → Delinquent → EvictionPending with one notice each, a job opened,
no movement, `home` still works; `@tenancy/resolve end` after a staff scene ends it); the same in
exact mode with a real obligation; `off` creates nothing and shows nothing; every stance's table
row; a stance flip mid-period applies next period; `Exploit` emits one front-attributed canon event
per period and `Conceal` none; housing pressure preempts an open opportunity and not vice versa;
expiry applies `StatusQuo` with no strain and no money; the per-faction pool caps at 8 open and
frees on resolution; an orphaned front renders and stops; a cadence jump evaluates once; the
economy never writes the sheet (assert `sheet:wod` unchanged across every test); chargen creates
the right artifact per mode; the Residence step offers only tier-appropriate vacancies and no
occupancy data; `@wod/economy migrate` in both directions mints nothing and hides/opens accounts as
specified.

---

## 6. Build discipline

As 08c §7: per-cut probe-confirmed adversarial verify before the next cut; clippy `-D warnings` and
the workspace tests; commit per cut; push on the owner's word. E5b starts only after E4b1 ships
(header gate); re-ground §4's E4 references against the built E4 first.

## 7. Amendments this plan fed back into E4 (08c)

1. **Cadence clocks ship in E4** — *merged* (08c Draft 0.4, `EconomicClock::Cadence`).
2. **Canon attestation matches a specific source** — *merged* (08c Draft 0.4, §6 item 2).
3. **Front attribution nests inside the kernel causal chain** once doc 09 C6 lands (`FrontRef` over
   `FactionId`/`FrontId` is the pre-C6 shape; only the action-local `PreparedCausalRef` exists in K
   today, `mush-core/src/action.rs:2265-2731`) — *merged* (08c Draft 0.5 §12).
4. **Title issuance for registered system principals** (`mush_economy::issue_title`, D-E3-13) —
   *added to 08c §6 as item 6 with this draft*; E5b consumes it.

## 8. Decisions

- **D-E5-1 A tenancy never ends automatically.** Automatic processing stops at `EvictionPending`
  with a bounded story window; ending, clearing the home anchor, and revoking access are explicit
  acts (answer 8). Duration-unbounded, notice-bounded and decaying, escalated per decision 25.
- **D-E5-2 Personal capacity is abstract by default; the income table is exact-mode-only**
  (answers 1–3). A `SupportLevel` (0..=5), not a balance, is what housing assesses.
- **D-E5-3 The economy never writes a sheet dot and never files trait-loss jobs** (answer 4).
- **D-E5-4 Access is enforced only through the movement/carry hooks; landlords have no implied
  entry; delinquency never restricts a tenant's own access** (answers 8–10; doc 08 §11.1's interim
  restriction step is dropped).
- **D-E5-5 `mush-housing` is game-neutral and sheet-blind; `ruleset-wod` interprets dots and
  submits `SupportPlan`s through housing's own entry, never a player command** (answer 17).
- **D-E5-6 A career is one source among several; stances alter pressure and opportunity, never
  income; stance changes apply next period** (answer 11).
- **D-E5-7 Haven dots are qualities, not housing tier or rent** (answer 15).
- **D-E5-8 Attention budget of one with housing-pressure preemption; the faction pool counts open
  operations; non-response is the status quo; every progress event names a front, orphaned fronts
  render and stop** (answers 18–20).
- **D-E5-9 Config is WoD-owned (`[wod.economy]`), parsed host-side; mode transitions are an
  explicit staff reconciliation that mints nothing** (answer 14).
- **D-E5-10 Dependents are deferred; `SupportSource::Dependent` is reserved** (answer 13).
- **D-E5-11 `Obj.home` follows the tenancy 1:1** — set on `Active`, cleared on `Ended` for every
  homed tenant and every guest/item homed inside — because core's destroy dispersal honours no hook.
- **D-E5-12 Property rooms are private ordinary rooms** — never sim spaces, presentation rooms, the
  chargen start room, thoroughfares, or shared with another property; a `DestroyGate` protects them
  while `Let`.
- **D-E5-13 Multi-tenant support is best-of** — any one resident's asserted support covers a shared
  lease; plans are per resident and cannot overwrite one another.
- **D-E5-14 Assessment and evaluation run on an E4 cadence clock**, never on an IC:OOC ratio or a
  literal calendar (answer 7).

## 9. Constants

`MAX_ROOMS_PER_PROPERTY` 16 · `MAX_FEATURES` 16 · `MAX_TENANTS` 8 · `MAX_GUESTS` 16 · `GRACE_PERIODS` 1 ·
`DELINQUENT_PERIODS` 2 · `UNKNOWN_GRACE` 1 · `WINDOW_RENOTIFY` 1 period · `MAX_WINDOW_NOTICES` 3, then
silent until escalation, then ≤ weekly · `WINDOW_ESCALATE` 2 periods · `MAX_ASSESSMENTS_PER_FIRE` 128 ·
`STRAIN_MAX` 10 · `COAST_OPPORTUNITY_PERIODS` 6 · `MAX_OFFSCREEN_OPS_PER_FACTION` 8 (open) · prompt
default expiry 2 periods · history and pending-notice caps 32 · **`PLAN_MAX_AGE_PERIODS` 2** (a
`SupportPlan` older than this is absent, not asserted — §3.5 S4) · **`RELAPSE_WINDOW_PERIODS` 2** (a
story window reopening inside this gap inherits the last one's notice budget — §3.5 S9).

## 10. The owner's answers (2026-09-03, verbatim summary — binding)

1. Hybrid with a narrower boundary: personal capacity abstract by default; organization money may
   be exact; asset title and tenancy state are exact. A major purchase does not automatically
   require exact personal cash — strain, financing, a contract, or a scene may serve.
2. Register `wod.usd` at scale 2; create or expose personal USD accounts only in exact mode.
3. Resources-to-income table in config, exact-mode-only; in abstract mode Resources maps to
   lifestyle/support and resilience. The cash table is not authoritative for the default game.
4. The economy never changes a sheet dot and never auto-files trait-loss jobs because a scheduled
   process ran. Delinquency → strain, status, story hook. Permanent changes via the approval path.
5. Properties are staff/builder-created ordinary rooms or room groups; a tenancy grants no `@dig`;
   players manage guests and approved features.
6. Named landlord organizations plus a generic `system:wod.housing` principal; abstract mode posts
   no pretend rent.
7. No coupling to an IC:OOC ratio or "the first of the month": a named campaign/domain clock with
   configurable cadence and bounded catch-up; periodic pressure evaluation, not a rent calendar.
8. No automatic termination. Automatic processing may reach `EvictionPending`, one private notice per
   transition, a bounded story window. Ending requires an explicit resolution after a scene or a
   deliberate staff waiver. A home must not disappear because a player was offline.
9. Tenant and tenant-managed guest access through movement/carry hooks; no automatic landlord
   entry when delinquent; inspection is an explicit policy term.
10. Leave `home` routing out; enforce through the existing `MoveAttempt` hook.
11. Four deterministic stances — Coast (stable, low pressure, few opportunities), Advance (more
    time demand/strain, advancement opportunities, no automatic dot gain), Exploit (temporary
    capacity at increased exposure/risk), Conceal (reduced visibility at reduced opportunity or
    capacity). A career is one source among inheritance, patron, criminal income, domain support.
12. Optional employer principals; exact-mode payroll from a real employer account or the faucet;
    abstract mode posts no routine payroll.
13. Dependents deferred (retainers, ghouls, Herd — Herd is not an economic dependent); reserve
    extension points only.
14. WoD-owned config `[wod.economy]` with `personal_mode`, `housing`, `careers`, `default_engagement`;
    no platform economy switch.
15. Chargen approval: `off` nothing; `abstract` a `SpendingProfile`; `exact` an account with starting
    funds. Haven dots describe exceptional qualities, never housing tier or rent.
16. Game-neutral `mush-housing`; no broad `RulesValue` refactor (core type already exists in the K
    worktree).
17. Housing never reads ruleset dots; `ruleset-wod` interprets and submits explicit plans.
18. Attention budget: at most one unresolved personal economic prompt per character; a bounded
    per-faction pool of off-screen operations.
19. Non-response chooses the conservative/status-quo default or leaves a story window pending;
    never a failed homework assignment.
20. Front attribution: progress must cite a specific front, opportunity, or operation.

## 11. Open questions (defaults chosen — **accepted by the owner 2026-09-03**; all three are binding as written)

1. `GRACE_PERIODS` 1 and `DELINQUENT_PERIODS` 2 on the cadence — three assessed periods from the
   first unsupported one to the story window.
2. Whether the chargen Residence step is mandatory when `housing = true` (default: optional).
3. Whether a landlord org's officers may create tenancies for their own properties (default: staff
   only in E5; `property.manage` covers show/list and guest-policy edits).

## 12. Review outcome (Draft 0.1 → 0.2)

Three Sonnet passes on 2026-09-03. Folded: **adversarial** — destroy dispersal honours no hook and
`Obj.home` was never tied to the tenancy (→ D-E5-11 and the Limbo test); `@teleport` cannot see the
issuing actor (→ scoped limitation, §1.1); thoroughfare rooms (→ D-E5-12 + `@property/rooms`
validation); no live destroy gate (→ `DestroyGate` + observer); single-slot multi-tenant plans (→
best-of map, D-E5-13); the faction pool never decremented (→ counts open operations); orphaned
`FrontRef`s (→ render and stop); no mode-switch path (→ `@wod/economy migrate`); prompt preemption
(→ housing pressure wins). **Design-fit** — title issuance unowned (→ 08c §6 item 6 +
`issue_title`); module order unstated (→ §2); the dropped interim-restriction step and pull-only
summaries made explicit (§1.1); daily-forever re-notify (→ decay, §3.2); landlord-side read and
per-role `+home` fields (§4); E5b's gate on E4b1 (header); the K3 precedent cited (§4); doc 08 §14.1's
stale abstract-rent example noted in doc 08. **Grounding** — chargen handler citation corrected to
`lib.rs:374-381`; the hook-bypassing `move_to` sites enumerated and scoped (§1.1); "08c §6.2" →
"§6 item 2"; §7 reframed (three amendments already merged, one new); host-side config plumbing;
`NamedResource` reusable but the renderer private; `wod-story` queue registration; ruleset-wod's
canon dependency added.
