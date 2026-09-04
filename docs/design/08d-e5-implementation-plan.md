# E5 implementation plan — World of Darkness housing and background support

**Status:** Draft 0.2 — reviewed (three passes: adversarial, code-grounding, design-fit against docs
08/08a/08c/09 and the owner's answers); every confirmed finding is folded below and itemised in §12.
**E5a is built** — `crates/mush-housing`, the pure library; read §3.5 "As built" for its signatures,
deviations and the seams E5b consumes. **E5b is built** — the `HousingModule` that attaches it to a
world; read §4.1 "As built" for its signatures, doc shapes, the pull-model assessment, the
lease-backed exact mode, the deviations and the handoffs to E5c. **E5c is built** — the WoD layer in
`ruleset-wod::economy` plus the host config plumbing; read §5.6 "As built" for the config shape, the
doc shapes, the evaluation's bounds, the sheet-blindness proof, the mode-transition table, the four
additive seams below the ruleset, and the handoffs to E6/E7.
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

Tests (planned ~30; **43 as built, 66 after the verify fold**, `crates/mush-housing/tests/e5b.rs`): scenario 23's housing half in abstract mode — supported periods are silent; an
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

### 4.1 As built (E5b, `crates/mush-housing` + the `mush-economy` seams it consumes)

Built as the world-attached half of the same crate. `cargo test -p mush-housing` **121 passing**
(54 E5a + 66 E5b + 1 in-crate; 98 before the verify fold); `cargo clippy --workspace --all-targets
-- -D warnings` clean; `cargo fmt` stays off, unchanged.

**Files.** `src/module.rs` (`HousingModule`, `HousingStore`, the keys, registration) ·
`src/boot.rs` (load + the four reconciliation passes) · `src/hooks.rs` (the
`MoveAttempt`/`CarryAttempt` hook, the destroy gate, the `ObjDestroyed` backstop, the reconnect
notice) · `src/tick.rs` (the pull-model assessment + the story-window decay) · `src/flow.rs` (the
world-attached halves of E5a's arrows: the `Obj.home` invariant, the story job, the per-transition
notice, and the ruleset's `submit_support_plan` entry) · `src/econ.rs` (the exact-mode
`EconomyObserver`) · `src/notice.rs` (the bounded offline queue) · `src/commands.rs` (the whole
verb surface) · `tests/e5b.rs`.

**Dependencies.** E5a's `{core, org, economy}` plus **`mush-jobs`** (a story window's bare `u64`
has to become a real ticket) and `mush-proto` (`RichText`, for the hook's refusal). Deliberately
**no** `mush-sim` and **no** `mush-config` edge — see deviation 3.

**Persistence** (namespace `housing`):

| doc | class | corrupt → |
|---|---|---|
| `property:{id}` | container | **fails the boot** |
| `tenancy:{id}` | container | **fails the boot** |
| `assess:{id}` | leaf (the pull cursor: `position`, `period`, `unknown_periods`, `seeded`) | skipped loudly, re-seeded |
| `notice:{obj}` | leaf (the bounded offline queue) | skipped loudly |
| `meta` | allocators + the tick's rotation cursor | tolerated field by field |

Both containers deserialize through E5a's `TryFrom` wires, so "parses" is the strong sense: every
§9 cap, the anchor-in-rooms rule, tenant ∉ guests and "a plan is filed under its own resident" are
re-checked on load. Allocators resume from `meta` **floored past every parsable key** and
saturating (E4b1's B1), so a hand-written `property:18446744073709551615` parks the allocator at
the ceiling — where minting refuses — instead of wrapping live ids back into use.

**Boot reconciliation**, exactly §4's list plus one re-derivation:

1. anchor or rooms missing / `GOING` → property `Withdrawn` + a level-2 page; **every tenancy on it
   untouched**.
2. a tenant object gone → dropped through `Tenancy::remove_tenant` (roster + plan + history entry,
   never `homed`, §3.5 A1/S11). The **last** tenant cannot be removed that way, so that case pages
   instead of ending anything.
3. an exact tenancy whose obligation is missing or cancelled → paged, status untouched, **never**
   `EvictionPending`.
4. the `Let`/`Vacant` latch is a projection, so boot re-derives it from the live tenancies.
   `Withdrawn` is never re-derived away — it is a decision, and only `@property/rooms` lifts it.

Capped at `MAX_BOOT_PAGES` (16), the economy's own rule.

**The pull-model assessment** (`src/tick.rs`). §4 says housing "subscribes to the E4 cadence
clock"; there is no subscription seam on a cadence, and adding one would put a housing callback
inside the economy's money tick. So it is the **pull** form of the same rule, and it needs no
economy change at all: housing keeps `last_assessed_position` per tenancy and, on its own
`TickCadence::Coarse(30s)` tick, assesses every tenancy whose cadence `position` has moved past it.
A missed fire loses nothing (the next one sees the same gap) and a duplicate fire does nothing (the
position is the idempotency key). One fire:

- takes at most `MAX_ASSESSMENTS_PER_FIRE` (128) tenancies from a **rotation cursor** (E4b2's B4),
  so a world with more than that still reaches every one of them;
- assesses **one period per tenancy however far the clock jumped** — forty periods of downtime
  assess once. That is E4's "a backlog is never charged" in its housing form: the ladder is three
  assessed periods long, and a restart must not spend all three while the player was asleep;
- **abstract** tenancies go through `assess_at(tier, plans, tenants, period, PLAN_MAX_AGE_PERIODS)`
  → `harden(a, unknown_periods)` → `advance`, with the per-tenancy `unknown_periods` run counter
  in `assess:{id}`;
- **exact** tenancies do not: *whether* they are behind is the obligation's answer, mirrored onto
  the ladder's first rung by the observer, while *how many assessed periods* they have been behind
  is a question only this cadence can answer, so `Grace → Delinquent → EvictionPending` is driven
  here. Reading one clock's elapsed time off another is the bug `EconomicDeadline` exists to make
  unwritable, and the split is exactly that rule;
- then drives the story window: `window_due` decides, the tick executes, and an `Escalate` mints
  the `EscalationId` **first** and folds it through `window_elapsed` (a bare `Escalate` is
  `NeedsEscalation`, never a skipped page).

A tenancy whose cadence record has gone away is **frozen, not advanced** — an unresolvable clock
never makes anything happen, the economy's own rule — and the cursor is seeded at creation, so the
first period the clock advances after a tenancy is created is the first one it is assessed on.

**Access** (`src/hooks.rs`). One `AccessHook` registered under **both** points, because E5a's
`may_take` *is* `may_enter`. The decision table, for a move into room `r`:

| question | answer |
|---|---|
| is `r` a room of a property this store knows? | no → allow (abstain) |
| is that property `Let`? | no → allow — a `Vacant` or `Withdrawn` residence is an ordinary room |
| does it have a live tenancy? | no → allow |
| `may_enter_with(t, &t.terms, who, inspector, is_staff, now)` | `Refused` → veto with one neutral text; anything else → allow |

`inspector` is the org/world question E5a cannot answer: an object principal is themself, an
organization is whoever holds `property.manage` on the property's compartment (or the org's root
scope), and a **system** principal is held by nobody, staff included — the `WorldAuth::acts_for`
rule. `now` is the tenancy's cadence position, or `i64::MIN` when the clock cannot be read, so a
clock error never opens a door. A **carry asks the same question twice**, of the room the object
comes out of and the room it goes into.

**Destroy.** A `DestroyGate` refuses `@destroy` of any room of a `Let` property, detailed for
somebody who may view it and `"You cannot destroy that right now."` for everyone else; the
`ObjDestroyed` observer is the backstop for destruction that never passes the gate, and marks the
property `Withdrawn`, keeps every tenancy as-is, pages level 2 and tells the tenants.

**Exact mode** (`src/econ.rs`), end to end:

1. `@tenancy/create <property>=<tenant…> cadence=<id> mode=exact rent=<n> from=<acct> to=<acct>`
   calls the new **`mush_economy::offer_lease`**: parties are the landlord and the tenants, the
   **landlord's acceptance is the offer** (a staff act on their behalf — exactly the
   `landlord_accepted` half of E5a's `accept`), the premises are pledged as the landlord's
   `Collateral` and the rent is an `Obligation` deliverable, with one
   `Attestation { by: Staff }` condition. The tenancy is written `Pending` with
   `Funding::Exact { contract, obligation: PENDING_OBLIGATION }`.
2. The **tenant's own `@contract/accept`** is the consent, and it is the last one, so the contract
   activates: the economy places `Encumbrance::Lease { lessor, contract }` on the premises and
   creates the rent obligation with `cause: Contract(id)`.
3. Housing binds that obligation and runs E5a's `accept`, so the tenancy goes `Active`, `Obj.home`
   points at the anchor and the property latches `Let` — through the same code path abstract mode
   uses.
4. Each period the sweep settles it. `ObligationChanged` mirrors Grace/recovery; the tick walks the
   rest.
5. A **terminal** contract ends the tenancy with `Resolution::Settled`. That is not an automatic
   ending: every route into a terminal contract state is somebody's explicit act, which is the
   resolution E5a reserved for exactly this (D-E5-1).

No housing code debits anyone anywhere: rent is settled by the economy sweep under the economy's
own authority, and housing stores two ids.

**The seams consumed in `mush-economy`** (§2 anticipated one; four shipped, all additive):

1. **`issue_title`** — as planned (§7 item 4). `@property/create` mints the anchor's title to the
   landlord under the wired issuer; `Obj.owner` of the rooms stays the builder.
2. **`open_escalation` / `acknowledge_escalation`** + `EscalationSubject::{HousingProperty,
   HousingTenancy}` + `EscalationKind::StoryWindow`. Housing pages through the **economy's** ladder
   rather than growing one of its own, so the dedupe, the reopen-after-ack, `@econ/alerts`, the
   renotify schedule and the GC are the ones a staffer already knows — 08a decision 25's whole
   point is one alert per decision, and two half-ladders is how that becomes two.
3. **`offer_lease` / `LeaseSpec`** — one call, because a lease is one act. The builder verbs are
   the **issuer's**, so a system landlord (`sys:wod.housing`, which types nothing) could not have
   built one at all, and every intermediate state (a pledge with no rent, rent with no premises) is
   something a tenancy must never be bound to.
4. **`@econ/cadence create|show|list|advance`** — E4b1 deviation 1's leftover, shipped here because
   E5 is the first cut with a clock a human has to name. `create <name> source=<real:<secs>|
   ic:<secs>|manual> [catchup=<n>]`, `show <id>`, `list`, and `advance <id> [<n>]` which is
   **`Manual`-only**: hand-advancing a real- or IC-sourced clock would put its `position` ahead of
   the source it derives from, and the next sweep would charge the same periods again.

One seam was added to **`mush-jobs`**: `set_job_status(ctx, id, by, status, note)`, the producing
module's counterpart to `open_job`. A story window that recovers must close the ST's ticket, or an
ST is left holding a decision about a home that is fine.

**Deviations and clarifications** (all recorded, none silent):

1. **`@tenancy/create` in abstract mode goes straight to `Active`.** §4's command list has no
   accept verb and §5.4's chargen approval "creates the tenancy (`Pending → Active`) through
   housing", so the staff act that creates it records both consents at once. Exact mode is the
   opposite by design: the tenant's `@contract/accept` is the consent, so it stays `Pending`.
2. **`@tenancy/resolve <id>=extend` resets the window at the module layer.** E5a has no `extend`
   arrow — its window functions only ever *spend* the budget — and adding a pure function that
   makes a window younger would contradict the decay rules it exists to enforce. So `extend` builds
   a fresh `StoryWindow` carrying the existing job, scene and escalation ids. **E5c handoff:** if a
   second caller ever needs it, it belongs in the library with the ids carried by construction.
3. **D-E5-12's "not a sim space / presentation room / chargen start room" is enforced without a
   `mush-sim` or `mush-config` edge.** §2 fixes housing's dependency set at `{core, org, economy,
   jobs}` and both of those would break it. Three mechanisms cover it instead: the **thoroughfare
   rule** (a sim space, a presentation room and a common area are all reachable by an exit from
   outside the property, so all three are refused — the anchor, the declared entrance, is the only
   exemption), a wiring-frozen **reserved-room list**
   (`HousingModule::with_reserved_rooms`, which is how the host passes the `[onboarding]` start
   rooms it parses), and housing's own **`no-property` marker**. Limbo is refused always.
   **E5c handoff:** wire `start_room` / `start_room_unapproved` into `with_reserved_rooms`.
4. **`@tenancy/resolve <id>=scene` is how the ST asks for the optional scene.** §4 says the window
   "optionally starts a scene when the ST asks via the job"; there is no callback from a job
   comment into a module, and inventing one would be a second dispatch path into housing. The ST
   asks by typing it, and the `SceneRef` recorded is opaque and boot-local by construction (E5a
   deviation 8).
5. **The story job's `opener` is `GOD`, explicitly.** The opener receives the job's status notices,
   and that must not be the tenant — this is an ST's ticket about their home, and the tenant has
   their own private notice. The queue's per-queue visibility decides who reads it.
6. **The tick period is 30 s wall clock**, which is a *latency* bound and nothing else: the pull
   compares cadence positions, so the tick's own rate never affects what is assessed.
7. **A `PENDING_OBLIGATION` sentinel (`ObligationId(u64::MAX)`).** `Funding::Exact` names both ids
   by type and the rent obligation does not exist until activation. `u64::MAX` is the one value
   that can never collide: the economy's allocators refuse to mint at the ceiling (E4b1's B2).
8. **The lease binds its obligation from `ObligationChanged`, not `ContractChanged`.**
   `ContractChanged { Active }` is emitted by the contract's own state write, which happens
   *before* the economy creates the obligations its deliverables describe — so the rent record does
   not exist yet at that edge. Creating it is what emits `ObligationChanged`, and that is where the
   binding happens; the tick re-checks it as the restart-safe catch-up.
9. **`landlord=housing` resolves to the server's own wired system principal**, never to a literal
   `wod.housing`. D-E5-5 makes this crate game-neutral, and a hard-coded World-of-Darkness name in
   it would be the one place that stopped being true; the principal the host registered as the
   title issuer *is* owner answer 5's "generic `system:wod.housing` principal", so naming it that
   way keeps the two the same by construction. `landlord=sys:<name>` still names one explicitly.
10. **New capability vocabulary** `property.view`, `property.manage` (`mush_housing::CAPABILITIES`,
    chained into the server's `org_capability_vocabulary`). Per §11 answer 3, binding:
    `property.manage` covers show/list and guest policy, and **never** tenancy creation, which is
    staff-only in E5.
11. **No state families, still.** E5a deviation 10 asked E5b to revisit doc 09's `action-state/*`
    lane for `tenancy:{id}`. It stays conventional: 08c §12's "K3 review" decision has not moved,
    the container-doc discipline already gives the failure mode the status ladder needs (a corrupt
    one fails the boot), and adding a typed family now would be a schema commitment made ahead of
    the review that owns it. **Carried to the K-series review, not to E5c.**

**Verify-fold deltas** (the shipped cut, amended — every one of these is a
`vf_<lens><n>_…` regression test in `tests/e5b.rs`):

- **One authority path for a landlord's officer** (A4/A5). `commands::officer_scope` resolves the
  compartment — the property's own when it names one, else the landlord org's **root scope** — and
  `officer_of` asks it for `property.view` **or** `property.manage`. `may_view_property`,
  `guest_only` and the access hook's inspector resolution all go through it, so a property created
  without `scope=` no longer locks its own officers out of the record while still letting them
  inspect it, and an invitation no longer demotes a manage-only officer to "just a guest". The
  hook's *capability* stays `property.manage` alone: inspection is a management right, and a read
  grant must not open a door.
- **The guest reduction is every read verb** (A1). `@property/show|list` and `@tenancy/list` join
  `+home` and `@tenancy/show`: a guest-only viewer gets the property's name and their own standing,
  never the landlord, tier, rooms, features, status or occupancy.
- **The assessment word, not the number** (A2). `@tenancy/show` renders `supported` / `strained` /
  `unsupported` / `unknown` to tenants and landlord officers; the numeric `SupportLevel`, the plan
  count and the sources are **staff only** — a `SupportLevel` is the Resources reading
  `ruleset-wod` derives from a sheet (D-E5-5), and best-of publishes the *stronger* co-tenant's band.
- **`landlord=` is not an org oracle** (A3). An unresolvable value, and a landlord for which the
  actor can view no property, both answer `No such property.`; it never widens to "no filter".
- **`+home/leave` and `+guest` never guess** (A6/H3). Both take the tenancy (`+home/leave <id>`,
  `+guest … home=<id>`) and refuse with a list when the actor lives in more than one place; a
  co-tenant's `Obj.home` is cleared **only when it resolves inside the property being left**, then
  re-homed on another live lease.
- **`every=` is a real key** (A7), documented as the rent period (default `DEFAULT_RENT_PERIOD_SECS`).
- **The access hook keys on the live tenancy, never the `Let` latch** (H1). `Withdrawn` is a
  service decision that lifts the *destroy gate*; it never unlocks a home somebody lives in, before
  or after a reboot. `HousingDestroyGate` keeps its `is_let` filter, so a withdrawn property can
  still be demolished once its tenancy ends.
- **`move_attempt` checks the source for things** (H2). A non-actor standing in an occupied home
  cannot be teleported out of it — the same fail-closed shape §1.1 already forces on the
  destination, since the hook cannot see who issued the move. People are exempt: nobody is trapped.
- **The anchor is exempt from the thoroughfare rule and nothing else** (H4). Every other check
  applies to it: Limbo, the `no-property` marker, the reserved list, a room another property holds,
  and — new — **any room carrying a `BehaviorBinding`**, which is the sim-space / interior-anchor /
  presentation-room test without the `mush-sim` edge §2 forbids. `mush-server` now wires
  `with_reserved_rooms` from `[onboarding]`'s two start rooms plus Limbo (deviation 3's handoff,
  landed here rather than in E5c).
- **The ending sweep walks transitively and scans for homes** (H5): the property's contents to a
  depth cap, plus a bounded pass over every object whose `Obj.home` resolves inside wherever it
  stands.
- **A destroyed room pages whenever somebody lives there** (H6), independent of the latch — the
  `ObjDestroyed` backstop and boot pass 1 both — because the economy's ladder dedupes by subject.
- **A relapsed window inherits `notified`, never `escalation`** (T1). Carrying the id made
  `window_due` answer `NotifyEscalated` forever, so a home that recovered and relapsed could never
  ask for a person again; re-paging reopens the same record (same id, `reopened` bumped), so it
  cannot double-mint. Same rule for `@tenancy/resolve <id>=extend`, which drops it too (T3).
- **The rent obligation debits a tenant** (T2). `offer_lease` refuses a `from=` account whose owner
  is not `PrincipalId::Object(o)` with `o` among the tenants, and refuses a system source by name
  (a standing mint); `mush_economy`'s `check_offerable` enforces the same rule generically for every
  `Deliverable::Obligation` — the debtor is a **party** and is never a system principal.
- **The causing actor is recorded** (T4). `close_window` / `close_pending_job` credit
  `acknowledge_escalation` and the job's resolution note to `ctx.actor` — the reinstating creditor,
  the resolving staffer, or the tick's `GOD` — instead of always `GOD`. A landlord-initiated
  recovery stays allowed: it is a recovery, not an ending.
- **Three new boot passes.** A `Pending` exact tenancy whose lease contract is missing or terminal
  is paged once with `@tenancy/resolve <id>=end` recommended, never ended and never
  `EvictionPending` (T5). A live tenancy whose cadence does not resolve is paged with the clock
  named, at boot and on every tick fire, and stays frozen (T6). A persisted `SceneRef` is cleared
  at boot, because core scene ids restart per boot (T8) — and `=scene` now checks that
  `attach_window` would accept **before** calling `ctx.start_scene`, so a second one leaks nothing.
- **The tick's working set is the unended tenancies** (T7). `HousingStore::open` is a derived index
  (`put_tenancy` and boot re-derive it from the status, never persisted), and `step` reads the
  status through the store borrow before cloning anything.
- **`BindEdge`** (T9) names whether "no rent obligation yet" is a fault: it is not on
  `ContractChanged { Active }`, where deviation 8 guarantees the record does not exist yet, and it
  is everywhere binding is due.

**Handoffs to E5c:**

- **`mush_housing::submit_support_plan(ctx, tenancy, resident, level, sources)`** is the whole
  interface (D-E5-5). The ruleset's period evaluation calls it in code; no player command does.
  Housing stamps both `asserted_at` (the cadence position) and `asserted_period` (its own monotonic
  ordinal) — a ruleset that supplied either could make a stale plan look fresh, which is what
  `PLAN_MAX_AGE_PERIODS` exists to prevent. A restatement that changes nothing writes nothing.
- **Restate every period.** A plan older than `PLAN_MAX_AGE_PERIODS` (2) is *absent*, not asserted,
  so an evaluator that stops running lets `UNKNOWN_GRACE` fire rather than propping the lease up
  forever (§3.5 S4).
- ~~**Wire the chargen start rooms** into `HousingModule::with_reserved_rooms` (deviation 3).~~
  **Landed in the verify fold (H4):** `mush-server` passes `[onboarding].start_room`,
  `start_room_unapproved` and Limbo. E5c extends the list, it does not create it.
- **The `[wod.economy].housing` knob** decides whether the server registers `system:wod.housing` as
  a title issuer at all; it is wired unconditionally for now, and is inert without housing.
- **Chargen's Residence step** creates the tenancy through the same `@tenancy/create` path
  (abstract → live immediately); `@property/list` already filters by view standing, so the step can
  offer vacancies without disclosing occupancy.
- **An org or system landlord has no inbox.** The delinquency notice reaches an *object* landlord
  only; E4c's observer seam is where an org's staff channel gets one.

### 4.2 E5b verify outcome (2026-09-03)

Three adversarial lenses over the shipped module, twenty-three findings, all confirmed by probe and
all folded with a regression test each (`tests/e5b.rs`, the `// ---- E5b verify fold ----` section).

**Lens A (command authority, existence hiding):** one HIGH — every guest read verb but `+home` and
`@tenancy/show` handed a guest the whole record; three MEDIUM — the sheet-derived support level
published to tenants and officers, `@property/list landlord=` as an org-existence oracle, and an
officer locked out of a property created without `scope=`; three LOW-MEDIUM/LOW — the invitation
demoting a manage-only officer, both player verbs silently picking the lowest-id tenancy, and a
dead `every=` key; one doc-only.

**Lens H (hooks, home safety):** one HIGH — `Withdrawn` cancelling every access rule on a home
somebody still lived in, which a single destroyed spare room was enough to trigger; three MEDIUM —
`@teleport` walking a thing out of an occupied home with no source check, `+home/leave` clearing
the wrong home, and the anchor exempt from every room rule rather than only the thoroughfare one
(with `with_reserved_rooms` never wired at all); two LOW-MEDIUM/LOW — the end sweep missing carried
and remotely-homed items, and a second destruction on a withdrawn property being silent forever.

**Lens T (tick, ladder, exact mode, durability):** three MEDIUM — a relapsed window that could
never page again, a rent obligation that could debit a non-party or a faucet, and `extend`
silencing the window it was meant to keep open; six LOW-MEDIUM/LOW — the ack credited to `GOD`, a
`Pending` tenancy invisible to boot, a vanished cadence freezing a home in silence, the tick
cloning every tenancy ever, `=scene` leaking a scene it could not re-attach after a reboot, and an
`error!` on the happy path.

Nothing in any lens found a path to `Ended` without a `Resolution`: D-E5-1 held throughout, and the
two findings that mattered most are again the ones where a *quiet* failure had replaced a loud one
(H1's master key and T1's permanently-silenced window). Gates: `cargo test -p mush-housing`
**121 passing**, `cargo test --workspace` **2674 passing across 182 suites**, `cargo clippy
--workspace --all-targets -- -D warnings` clean.

**Handoffs to E5c** (in addition to §4.1's):

- **Extend `with_reserved_rooms`.** It is wired from `[onboarding].start_room` and
  `start_room_unapproved` plus Limbo; E5c should add whatever else the ruleset knows is never
  somebody's living room (domain presentation rooms it names explicitly, festival grounds, and so on).
- **The assessment word is the tenant-facing vocabulary.** `+resources` and the chargen Residence
  step must render the word (`supported`/`strained`/`unsupported`/`unknown`), never the numeric
  `SupportLevel`, to anyone but staff — the split is enforced in `@tenancy/show` and has to be kept
  by every new render that touches funding.
- **The tick's live-id index.** `HousingStore::open_tenancy_ids` is the working set; any E5c code
  that iterates tenancies periodically should use it rather than `tenancies()`, and any new writer
  must go through `put_tenancy` so the index stays derived.

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

Tests (planned ~32; **40 as built**, **60 after the §5.7 verify fold**, `crates/ruleset-wod/tests/e5c.rs`): scenario 23 end to end in abstract mode (Resources 3 quietly supports a tier-2
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

### 5.6 As built (E5c, `ruleset-wod::economy` + the host config plumbing)

Built as a submodule of the ruleset (`crates/ruleset-wod/src/economy/`), plus four small additive
seams in crates below it. `cargo test -p ruleset-wod -p mush-housing -p mush-economy -p mush-server`
**923 passing**; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test
--workspace` **2735 passing across 184 suites, 0 failing**. `cargo fmt` stays off, unchanged.
(After the §5.7 verify fold: **943** and **2755 across 184 suites**, clippy still clean.)

**Files.** `src/economy/config.rs` (`WodEconomyConfig`, `PersonalMode`, `Engagement`, the three
faucet names and `wod.usd`) · `record.rs` (§5.2/§5.3's records + the §9 constants) · `store.rs`
(`WodEconomyStore`, the four documents, boot `load`) · `boot.rs` (the clock, the instrument, the
faucet accounts) · `evaluate.rs` (the period evaluation) · `prompt.rs` (the attention budget,
preemption, the pool, non-response, front rendering) · `chargen.rs` (the Residence step + the
approval artifacts) · `migrate.rs` (`@wod/economy migrate`) · `commands.rs` (the verb surface) ·
`tests/e5c.rs`.

**Dependencies added to `ruleset-wod`:** `mush-housing`, `mush-economy`, `mush-org`,
`mush-faction`, `mush-canon` — exactly §2's list. None of the five depends on any ruleset crate,
directly or transitively, so the edge introduces no cycle (checked over the full closure).

**Config shape** (`[wod]`, parsed host-side in `mush-config` → validated in
`mush_server::config_ext::wod_economy_config` → `WodRuleset::economy`):

```toml
[wod.economy]
personal_mode = "off"            # off | abstract | exact          (default: off)
housing = true                   # create tenancies; register the title issuer
careers = true
default_engagement = "ambient"   # ambient | engaged
cadence = "chronicle.period"     # the campaign clock, by NAME
starting_funds_major = 0         # exact only
reserved_rooms = []              # extra #N rooms that may never be property rooms

[wod.economy.income]             # exact only, authoritative only there
"3" = 1200                       # major units per period, per Resources dot
```

An unknown `personal_mode` or `default_engagement`, an income key outside `0..=5` and an empty
`cadence` are **hard boot errors**, on `[rulesets].enabled`'s rule that a silently-ignored typo is
worse than not starting.

**Doc shapes** (namespace `wod`, alongside M17's `conceal`/`pending`/`extended`):

| doc | class | corrupt → |
|---|---|---|
| `profile:{obj}` | leaf | **skipped loudly and re-derived in the same boot** (`economy::boot::run`) |
| `eval:{obj}` | leaf (the pull cursor: `position`, `period`, `seeded`) | skipped loudly, re-seeded |
| `residence:{obj}` | leaf (a chargen pick, consumed at approval) | skipped loudly |
| `econ:meta` | allocators (`next_prompt_id`, `eval_cursor`) | tolerated field by field |

A corrupt profile does **not** fail the boot the way housing's two container docs do, and that is a
decision rather than an omission: a `SpendingProfile` is *derived* — Resources comes off the sheet,
the level comes off Resources, and the plan it submits is restated every period — so re-deriving one
costs the character their strain counter and their open prompt and costs their **home** nothing.
Both losses are the lenient half of every rule they take part in. The prompt allocator floors past
every id a surviving profile holds, not only past `meta` (E4b1's B1), and saturates.

"Costs their home nothing" is true **because the boot re-derives it**, not because the tick would
(verify fold T1): the evaluation's working set *is* the profile map, so a skipped document used to
be a character who silently left the economy, submitted no plan, and walked to `EvictionPending` on
`UNKNOWN_GRACE`. `economy::boot::run` rebuilds one for every character carrying an `eval:{obj}`
cursor or a live tenancy without a profile beside it, from the sheet exactly as
`migrate::create_profiles` does, re-seeds the cursor at the clock's current position, and re-points
the wallet. A `profile:{obj}` whose own `obj` disagrees with its key is a **loud skip** as well
(fold T2) — installing it would have made `step(#5)` read #5's sheet and write #7's record at every
boot forever, which is the shape `mush-housing` refuses outright.

**The evaluation's bounds** (`evaluate.rs`, `TickCadence::Coarse(30 s)`, tag `wod.economy`):

- at most `MAX_EVALUATIONS_PER_FIRE` (128) characters per fire, from a **rotation cursor**, so a
  world with more than that still reaches every one of them;
- **one period per character however far the clock jumped** — forty periods of downtime evaluate
  once. Catch-up is computed from the cadence `position`, never from a tick count: the position is
  the idempotency key, so a missed fire loses nothing and a duplicate fire does nothing. The 30 s
  wall period is a *latency* bound and nothing else;
- the cursor is **seeded at profile creation** at the clock's current position (the ruleset-side
  twin of `flow::seed_assessment`), so the next period the clock advances is the first one this
  character is evaluated on. This is load-bearing, not cosmetic: housing assesses *before* the
  ruleset does (module order), so a wasted first period is a period the tenancy has no plan at all,
  and two of those in a row is exactly what `UNKNOWN_GRACE` fires on;
- a cadence that does not resolve **freezes** the evaluation and says so loudly, once per fire — an
  unresolvable clock never makes anything happen (the economy's rule, and E5b's);
- every match in `evaluate.rs` is exhaustive, and the file contains **no** `?`, `unwrap`, `expect`
  or panicking index at all — a tick handler runs in the world task, so a panic here takes the world
  down. `prompt.rs` and `store.rs` use `?` only to short-circuit an `Option`-returning helper
  (`expire`, `alloc_prompt`), and `expect` only on infallible serde of owned plain data inside a
  persist helper — the same idiom `mush_housing::put_tenancy` and `mush_economy::persist_*` use.
  Nothing in the layer can panic on record data.

**The sheet-blindness proof.** The whole `economy` module reads the sheet in **one** function,
`economy::sheet_resources` (plus `sheet_haven`, the same shape), and `SHEET_ATTR` appears nowhere
else in it. `World::set_attr`, `SheetStore::put_by` and `mush_ruleset::validate` appear **zero**
times in `src/economy/**` — the layer has no write path to a sheet at all, and no jobs-filing path
either (it opens no job of any kind). What crosses into `mush-housing` is a
`SupportPlan { level: 0..=5, sources: [label] }` and nothing else; `SupportSource::to_housing` is
deliberately lossy, dropping the stance, the employer, the front and the risk numbers. §5.5's
"assert `sheet:wod` unchanged across every test" is pinned three ways: a twelve-period run with an
`Exploit` career and answered prompts, an `Advance` run, and scenario 23's whole ladder.

**The mode-transition table as implemented** (`migrate.rs`, `@wod/economy migrate`, staff, one-shot,
idempotent, and it lists what it did):

| flip | what it does | what it never does |
|---|---|---|
| → `abstract` (from exact) | creates a `SpendingProfile` per **approved** sheeted character from Resources (fold M2; the skipped are named in the report); **stops** every income line it opened; reports which wallets `+resources` now hides | touch an account or a balance — they are economy infrastructure and persist untouched |
| → `exact` (from abstract) | opens a **zero-balance** `wod.usd` account per profiled character and points the profile at it | mint anything — the report says so and names `@econ/mint` as the attributed way to seed one |
| `housing = false` | stops **new** tenancies (chargen creates none; the title issuer is not registered) | end, alter or hide an existing tenancy |
| `off` | stops evaluating; stops the standing mints | delete a profile, a wallet, a balance or a tenancy |

**Deviations and clarifications** (all recorded, none silent):

1. **`personal_mode` defaults to `off`, `housing` to `true`.** §5.1's block *illustrates*
   `abstract`; making that the default would switch a live E5b server into a mode it never
   configured. Off-config the layer registers its verbs, creates nothing, evaluates nothing and
   shows nothing — the byte-identical back-compat rule the rest of the tree follows — while
   `housing = true` keeps the title issuer wired exactly as E5b wired it.
2. **`CareerProfile::pending_stance`** joins §5.2's struct. "A stance change takes effect at the
   **next** period boundary" cannot be implemented without somewhere to hold the pending answer;
   `+career/stance` writes it, and the evaluation's first act is to land it.
3. **`Engagement` is what gates opportunity generation.** `Ambient` (the default) never opens an
   `Opportunity` or `Advancement` prompt; the career still supports the lease, still carries strain
   and still leaves `Exploit`'s evidence. `Engaged` is opting in to being asked. §5.2 lists
   `engagement` without saying what it does, and this is the reading that serves the tedium axis.
4. **`Advance` emits no canon evidence.** §5.2's table calls its exposure "low"; the two rows the
   plan pins to a *count* are `Exploit` (one per period) and `Conceal` (none). Inventing a rate for
   "low" would have meant inventing a constant §9 does not have, so `Advance`'s exposure is the
   `+career` visibility line and nothing in canon.
5. **A career with no front generates nothing at all.** Owner answer 20 says every opportunity and
   every progress event cites a specific front; the honest consequence is that an unattributed
   career is quiet by construction rather than generating unattributed prompts. `@career/set`
   records a front that does not resolve yet (a staffer may be setting one up ahead of the faction)
   and the evaluation simply stays quiet until it does.
6. **`Advancement` prompts count against the per-faction pool.** §5.3 says the pool counts
   "opportunities … attributed to a faction's front"; an advancement prompt is one, so both
   front-bearing kinds are counted and `StrainRelief`/`HousingPressure` are not.
7. **The pool is derived, never stored.** `WodEconomyStore::open_ops()` recomputes it from the live
   prompts on every ask, so "decremented on resolution or expiry" is true by construction and there
   is no counter to drift. It is the seam the faction simulation shares.
8. **A prompt that lapsed this period is not replaced this period.** The point of the budget is
   quiet; answering nothing must not summon the next question in the same breath.
9. **The Residence step does not go through `may_view_property`.** Housing's read verbs answer
   `No such property.` to anybody who is not a tenant, guest, officer or staff — which is every
   character in chargen — so the step applies its own narrower rule: only a property that is
   `Vacant` **and** carries no live tenancy is offered, and only its **name and tier** are printed.
   A `Let` or `Withdrawn` property is not listed at all, so occupancy is not inferable from what is
   present *or* from what is missing. A pick that is not on the offered list is refused with the
   same words a nonexistent one gets.
10. **Chargen's starting funds are attributed to the approving staffer.** Nobody acts for a `System`
    principal, staff included (E4b1's A-lens rule), so a plan whose *actor* is `system:wod.chargen`
    is refused at validation. The post debits the chargen faucet's account with
    `actor = PrincipalId::Object(ctx.actor)` and a deterministic idempotency key — the same
    attribution `@econ/mint` carries, and a re-run of the handler mints nothing new.
11. **The approval artifacts never fail the approval.** A sheet a storyteller has signed off is not
    un-signed because an account could not be opened; whatever went wrong is reported to the
    approver instead.
12. **`+resources` shows the number to the character themself and to staff, and refuses everybody
    else outright** — E5b fold A2's rule, extended by the brief. A non-staff third party cannot ask
    about somebody else at all, so there is no assessment-word branch for one (the first clause used
    to promise a word branch its own third sentence made unreachable; verify fold D3 deleted the dead
    arm rather than keeping a render whose only helper, `housing_word`, carries no view predicate).
    The exact-mode balance is rendered only when `mush_economy::may_view_account` says yes, and is
    omitted entirely otherwise (never a placeholder).
13. **Two new documented commands are `@wod/economy` and `@career/set`** — `@wod` with an `economy`
    switch and `@career` with a `set` switch, matching `@econ/cadence`'s shape. `+chargen/residence`
    is the classic-surface twin of the wizard's Residence step so the stage is drivable without an
    input context.
14. **No state families, still.** E5a deviation 10 and E5b deviation 11 carried the `action-state/*`
    question to the K-series review; E5c adds nothing to it.

**Verify-fold deltas (2026-09-04).** What §5.7's fold changed about statements §5.6 already makes.
Everything else above stands as written.

- **The income row is keyed on the Resources dot, never the stance-derived level** (M1).
  `maintain_income` rated the standing mint off `profile.level` — `resources ± stance` — so
  `+career/stance exploit`, an ungated player verb, moved the mint one whole income row up on the
  same `ObligationId`. D-E5-6 stands exactly as written; it was the code that was out of line with
  it, with §5.1's "one amount per dot" and with `+career`'s own wording. `profile.level` is still
  what `submit_plans` hands housing, which is where §5.2's table delivers Exploit's temporary
  capacity.
- **`migrate`'s population is the *approved* sheeted characters, and the skipped are reported**
  (M2). `+chargen` writes `sheet:wod` at `ApprovalState::Unapproved` the moment a player starts, so
  the old presence test banked a character who typed their own `backgrounds:Resources=5` and never
  submitted. The approval is read through the ruleset's own `Dispatch` — the table
  `chargen_approval` stamps — so the two readers cannot disagree. This restores §5.4 and answer 15's
  asymmetry: chargen gives a character their artifacts **at approval**; migrate is for the
  population that predates the flip.
- **A `profile:{obj}` the loader skips is re-derived in the same boot** (T1), which is the only
  reason the corrupt-profile rationale's "costs their home nothing" is true; and **a profile doc
  whose `obj` disagrees with its key is skipped loudly** (T2). Both are recorded in the doc-shapes
  table above.
- **A prompt or `Coast` stamp from a *future* ordinal is stale, not fresh** (T3). `expired` and the
  `COAST_OPPORTUNITY_PERIODS` check both guard the subtraction, so a re-seeded cursor cannot give a
  prompt seven periods of life in a two-period slot, or silence `Coast` for six periods it never
  spent. This is `mush_housing::assess`'s clock-moved-backwards decision, made explicitly here.
- **A live `wod.income` line that no profile names is adopted or cancelled at boot, and a hand
  cancel is a decision the tick does not undo** (M3, M4). `SpendingProfile::income` was the layer's
  only name for the obligation; it is now a **cache**, because the record is self-identifying by
  `cause` + `minted_by` + `credit_account`. The pointer is no longer cleared after a *failed*
  cancel (which had stranded a live mint and let the next period open a second), `open_income`
  adopts rather than duplicates, and `economy::boot::run` sweeps the rest. A staffer's or the
  creditor's `@obligation/cancel` sets `SpendingProfile::income_cancelled_at` (the Resources dot it
  was taken at) and opens nothing in its place until that dot changes or `@wod/economy migrate`
  runs — E5b's `Withdrawn` rule, one record class over. `income_cancelled_at` is the one new field
  the fold added, in deviation 2's shape: a rule about a decision needs somewhere to hold it.
- **`careers = false` really does stop stances** (D2). Steps 1–3 of the evaluation applied
  `level_delta` and `strain_delta` unconditionally, and `career_cmd` gated only on the mode, so a
  careers-off server still sold a support tier for a typed stance. The evaluation now takes the
  stance as `Coast`, and `/stance`, `/engage` and `/source add career=` refuse in the off-mode
  gate's own words.
- **The chargen stake is bounded by `starting_funds_major` alone; the cap sentence covers income
  *lines*** (M5), and **an over-cap income row is marked on the `@wod/economy` screen** (M6). Neither
  is a new refusal: deviation 10 puts the stake in `@econ/mint`'s uncapped attributed class and
  deviation 11 forbids failing an approval, and §5.6 does not list an over-cap row among the hard
  boot errors. Both are now *said* — a boot `warn!` for the stake, and the `DOES NOT RESOLVE`
  treatment for the row.
- **The chargen Residence pick is re-checked at approval** (M7). `vacancies` filtered from the live
  sheet at pick time, so dots moved before `+chargen/submit` could hand a character a home their
  approved sheet cannot carry. `settle_residence` recomputes the level from the approved sheet,
  drops a pick that no longer fits, and reports the property's tier and the character's level to the
  approver — deviation 11's shape, report and never un-sign. Both outcomes print the tier and level.
- **A front renders only to a viewer `faction_listed` admits** (D1). `render_front` gated on
  existence alone and then printed a faction's name and a front's title verbatim, which is the
  module header's own existence-hiding rule honoured for property, tenancy and account and skipped
  for fronts. Names resolve only through `FactionState::faction_listed`; everything else falls back
  to `FrontRef::raw()`'s bare ids, so "you may not see it" and "there is no such faction" read
  identically. Attribution on the record is unchanged (answer 20) — only the rendering narrows.
- **Deviation 12's first clause is corrected to match its third** (D3), and the dead third-party
  word branch is gone.
- **`@career/set` says when a front or an org employer does not resolve** (D4), `+career`'s help
  and its `domain=` refusal agree with the parser without becoming an org oracle (D5), and
  `@wod/economy` matches its argument exhaustively (D6).
- **Documented, not changed:** the `HousingPressure` prompt opens at `Delinquent`, because the
  `Grace` rung is served by housing's own private notice (T5). §5.3's "housing's own notices are
  notices, not prompts" and §11 default 1's one-period cushion are the reason; `behind_tenancy` now
  says so, and a test pins the rung so it cannot drift.

**The four additive seams below the ruleset** (all `pub`, all with the same locks their neighbours
carry, all consumed by exactly one new caller plus the staff verb that already existed):

1. **`mush_economy::create_cadence(ctx, name, source, period, catch_up)`** + `CadenceError`,
   `MAX_CADENCES`, `MAX_CADENCE_NAME_BYTES`. `@econ/cadence create` now calls it, so the caps, the
   duplicate-name rule and the "anchored at now, not at the epoch" rule are one implementation. A
   ruleset's boot is the other caller — a campaign clock named in configuration has to exist before
   the first period is assessed on it, and there is nobody to type the verb at boot. `CadenceError`
   is a **dedicated** error rather than a new `EconomyError` variant: creating a clock moves no
   value, and `sweep::disposition` classifies every `EconomyError` by name with no `_` arm, so
   widening that enum would force a settlement-disposition decision that does not exist.
2. **`mush_economy::set_system_obligation(ctx, faucet, id, Option<Amount>)`** — the faucet's own
   maintenance of a standing mint it created: `Some(a)` re-rates, `None` cancels.
   `create_system_obligation` opens the hole and something has to be able to close it; a character
   whose Resources drop is owed the smaller stipend from the next period, and a deployment that
   flips back to `abstract` must stop minting. The four locks are creation's, re-asked: the record
   must be one **this faucet minted** (`minted_by`), still live, with both accounts `Active` and
   the debit account still the faucet's, and the new amount inside the live `mint_cap`. A missing
   record and one minted by somebody else answer the same `Unauthorized`, so it is no oracle. Only
   the amount or the status moves — not the schedule, the grace policy, the counters or the
   history — so a re-rate is not a re-anchor.
3. **`mush_housing::create_tenancy(ctx, property, tenants, cadence, terms)`** — the abstract half of
   `@tenancy/create`, lifted so chargen approval and the staff verb share one creator. Six
   preconditions travel with it (property exists, not `Withdrawn`, no live tenancy, cadence
   resolves, the tenant caps, and E5a's `accept`), plus the seeding, the `Obj.home` invariant and
   the `Let` latch. Exact mode stays in the command: a lease's activation is the tenant's own
   `@contract/accept`, which is not something a caller can do on their behalf. Six new
   `HousingError` variants name the refusals; `@tenancy/create` folds each through `hidden(..)`, so
   the command's existence hiding is unchanged.
4. **`mush_housing`'s re-exports widen** by `officer_of` and `render_principal` — reuse rather than
   fork. `+career` renders an employer principal through housing's own renderer, and `officer_of`
   is the one landlord-authority path (E5b fold A4/A5) that any later layer needing it must go
   through rather than re-deriving. The tick's live-id index (`open_tenancy_ids`) needed no new
   export: E5c asks the per-character `live_tenancies_of_tenant` and never walks the store.

**The host wiring changed shape once.** `server_modules_with` takes
`wod: Option<&WodEconomyConfig>` and `ruleset_modules` takes `&WodEconomyConfig`. 08d §2's "the
server registers `system:wod.housing` **only when the WoD ruleset is configured**" is now real: the
title-issuer list is empty when `[rulesets].enabled` does not select `wod` or
`[wod.economy].housing = false`, and `@property/create` then refuses rather than minting title
nobody asked for. `HousingModule::with_reserved_rooms` is **extended**, not re-wired —
`[onboarding]`'s two rooms plus Limbo (E5b fold H4) chained with `[wod.economy].reserved_rooms`.

**IC time.** The evaluation reads the K4a versioned world timeline through `Ctx::world_ic_now()`
where it needs a stamp — the whole `ClockStamp`, generation and all, is serialized into an exposure
record's `detail` and onto a `Prompt`, so nothing downstream can compare two of them across a rate
change by accident. E5c **registers nothing** with the timeline and owns no clock of its own.

**Handoffs to E6/E7:**

- **The per-faction pool is the seam the faction simulation shares.** `WodEconomyStore::open_ops()`
  is a `BTreeMap<FactionId, u32>` derived from the live prompts; E5 counts only its own. When the
  faction sim starts running operations of its own, it should read (and contribute to) this map
  rather than growing a second budget — the whole point of `MAX_OFFSCREEN_OPS_PER_FACTION` is one
  ceiling per faction, and two half-ceilings is how that becomes none.
- **`FrontRef` is the pre-C6 shape** (08d §7 item 3). Once doc 09's C6 causal chain lands, front
  attribution nests inside the kernel's own chain and `FrontRef` becomes a projection of it; the
  canon `source` string (`front:{faction}:{front}`) is the stable part and is what E4c matches an
  attestor on, so it should survive the change.
- **`wod.career.exposure` is a canon record, not an attestor.** It is deliberately **not**
  registered through `EconomyModule::with_canon_outcome_kinds`, because a contract gated on
  evidence its own subject produces is one they can settle for themselves (E4c fold E1's rule).
  E6b's `scene.outcome` is the shape a real attestor takes.
- **Dependents remain reserved** (D-E5-10): `SupportSource::Dependent` exists in both the ruleset's
  enum and housing's, and nothing constructs or interprets either.
- **`@wod/economy migrate` is the pattern for every later mode flip.** It is a staff act, one-shot,
  idempotent, it lists what it did, and it mints nothing in either direction.


### 5.7 E5c verify outcome (2026-09-04)

Three adversarial lenses over the shipped layer, twenty-two findings: **twenty confirmed and folded
with a regression test each** (`tests/e5c.rs`, the `// ---- E5c verify fold ----` section — seven
under lens M, seven under lens T, six under lens D), two refuted by the refutation panel and
recorded rather than fixed.

**Lens M (money, the standing mint, the housing seam):** two HIGH — a player-typed stance re-rating
the standing mint (`maintain_income` keyed on the stance-derived level, so `+career/stance exploit`
moved a character 500 → 5,000 major per period on the same `ObligationId`, and `conceal` cancelled
it outright), and `@wod/economy migrate` profiling, banking and paying an **unapproved** sheet (the
population test was the presence of `sheet:wod`, which `+chargen` writes at `Unapproved`); one
MEDIUM — an orphaned mint nothing names, where the profile pointer was the layer's only name for
the obligation, both cleanup paths dropped it *after a failed cancel*, and the recovery path then
opened a second live line into the same wallet; four LOW — a hand `@obligation/cancel` silently
replaced at the next period, a chargen stake bounded by nothing but its own config key, an income
row above the mint cap printing as live while paying nobody, and a Residence pick validated at pick
time only.

**Lens T (the tick, durability, the prompt):** one HIGH — a dropped `profile:{obj}` that nothing
ever re-derived, because the evaluation's working set *is* the profile map, so the character left
the economy silently and their home walked to `EvictionPending` on `UNKNOWN_GRACE`; three MEDIUM —
a profile doc whose `obj` disagreed with its key (which would have overwritten another character's
record at every boot forever), a prompt stranded in an old ordinal by a re-seeded cursor (seven
periods of life in a two-period slot, holding the one attention slot D-E5-8 reserves for housing),
and a `GOING` character still evaluated and still committing front-attributed canon; two LOW plus
one INFO — a lapse line that never counted down, the `Delinquent` rung (documented, not changed),
and no loader or reboot coverage at all.

**Lens D (disclosure, authority, the verb surface):** one HIGH — `render_front` naming a hidden
faction and a staff-only front title to a non-member, on `+career` and `+prompt`, because it gated
on existence rather than on `faction_listed`; one MEDIUM — `[wod.economy].careers = false` switching
nothing off on the evaluation's first three steps or on the player verbs, so a stance still bought a
support tier; three LOW and one INFO — a dead third-party disclosure branch that would have leaked
the moment it was revived, `@career/set` recording an unresolvable front in silence, a `domain=`
help string that disagreed with its parser, and `@wod/economy` swallowing an unknown argument.

The two folds now share their headline: **the findings that mattered most are again the ones where a
quiet failure had replaced a loud one** — the orphaned mint, the un-re-derived profile, and the
silently-resurrected cancel. The E5c-specific one is narrower and sharper: **every money defect in
the layer traced back to a number keyed on the wrong field** — the stance-derived level instead of
the sheet's dot, an unapproved sheet's dots instead of an approved sheet's, and a pointer that was
the only name for a mint instead of one cache among several.

Gates as run: `cargo test -p ruleset-wod -p mush-housing -p mush-economy -p mush-server`
**943 passing** (923 before the fold, plus the twenty regression tests); `cargo clippy --workspace
--all-targets -- -D warnings` clean; `cargo test --workspace` **2755 passing across 184 suites, 0
failing**. `cargo fmt` stays off, unchanged.

**Refuted, not folded.** Two findings the refutation panel rejected, recorded here so the reasoning
is not re-litigated:

- **`+prompt <option>` answering a prompt whose life is already over** — refuted 3–0 on arithmetic.
  Prompt expiry is measured in the character's own monotonic `EvalState::period`, never in cadence
  positions, so `@econ/cadence advance <c> 10` consumes no prompt life at all; `expire` runs
  unconditionally in the same pass that increments `period`, so `choose` cannot be handed an expired
  prompt. The strain observed was `opportunity_options()`'s documented `Strain(1)` for an
  affirmative answer to a **live** prompt, beside a free `pass`.
- **`min_role: Role::Royalty` locking `SIM_ADMIN` staff out of `@wod/economy` and `@career/set`** —
  refuted 2–1. The Royalty floor is the ruleset crate's uniform convention (all seven `min_role`
  sites in `crates/ruleset-wod/src/`, and `ruleset-shadowrun` identically); the `SIM_ADMIN ∨
  Royalty` handler gate is the *module* layer's convention, and E5c sits on the ruleset side of that
  boundary. Reachability seals it: `Powers::SIM_ADMIN` is held implicitly by Wizard+ (already ≥
  Royalty) or explicitly through `Obj.powers`, which no production code path writes. Worth tidying
  if a power-grant verb ever lands; today it grants nobody access they should not have.

**Handoffs to E6/E7** (extending §5.6's four):

- **A boot reconciliation pass is now the third of its kind**, after 08c §4.8's B2 orphan-escrow
  sweep and E5b's tenancy reconciliation. A later layer that mints an E4 record or points at one
  owes itself the same pass, and the pattern is the load-bearing part: **the record is
  self-identifying by `cause` + `minted_by`, so the pointer is a cache, not the name.** A pointer
  that is the only name for a record strands the record the moment the pointer is lost — and the
  cheapest way to lose one is to clear it after a write that failed.
- **`render_front` is now viewer-aware, and that is the seam the faction simulation must keep** when
  it starts rendering its own operations. One predicate (`FactionState::faction_listed`), not two:
  a second, kinder rule somewhere else is how a hidden faction becomes visible by accident.
- **`careers = false` is the precedent for every later partial switch.** A config field's doc
  comment is a contract — "whether careers, stances, opportunities and exposure are evaluated" has
  to mean all four — and a switch that only half-applies must be recorded in the mode table the way
  `housing = false` spells its own limit out.
- **`+prompt`'s countdown is the first player-facing read of `EvalState::period` outside the tick.**
  Any later view that quotes a deadline reads the character's own ordinal, never the constant the
  deadline was configured from; a write-once constant interpolated into a countdown is a number that
  is right exactly once.

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
