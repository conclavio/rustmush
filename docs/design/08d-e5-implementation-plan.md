# E5 implementation plan — World of Darkness housing and background support

**Status:** Draft 0.1 — pre-build plan, awaiting the three-pass design review. Companion to
[08-economy-organizations-ownership.md](08-economy-organizations-ownership.md) (§6.3 abstract wealth,
§11.1 the WoD slice, §18-E5), [08a-economy-build-status.md](08a-economy-build-status.md) (ledger),
[08c-e4-implementation-plan.md](08c-e4-implementation-plan.md) (E4, which E5 is layered on), and the
owner's twenty answers of 2026-09-03 recorded verbatim in §10 — **they are the design constraints
and override doc 08 §11.1 where the two differ.** Grounded in the tree at `1b4da03` by two surveys
(the WoD sheet/ruleset API; property/access/story seams); every `file:line` was checked. Departures
from doc 08 are numbered decisions (D-E5-n, §8).

> Timing: E5 depends on E4's obligations, contracts, and cadence clocks, which are planned (08c
> Draft 0.4) but unbuilt. This plan targets E4's *planned* interfaces and must be re-grounded once
> E4 lands. Two E5 requirements fed back into 08c: economy-owned **cadence clocks** (E4 had deferred
> named clocks) and **source-specific canon attestation** (§7).

---

## 1. Scope — what E5 is, in the owner's terms

E5 makes a World of Darkness character's background quietly support housing and ordinary life until
a missed obligation becomes a story hook (doc 08 §1), under a **narrow exact/abstract boundary**:

- **Personal monetary capacity is abstract by default.** A character has a *support level* derived
  from Resources and its sources, not a wallet. Routine life posts nothing. A "major purchase" is
  not automatically an exact cash transaction — it may cost strain, need financing, a contract, or a
  scene (owner answer 1).
- **Organization money is exact** (E2/E3 as shipped). **Asset title and tenancy state are exact**
  (records, not vibes).
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
  unresolved personal economic prompt per character; a bounded per-faction pool of off-screen
  operations), **non-response semantics** (ignoring a prompt chooses the conservative default or
  leaves a window pending — never a failed homework assignment), and **front attribution** (every
  economic or faction progress event cites a specific front, opportunity, or operation) (18–20).

**Exit criterion (doc 08 §18-E5, adjusted by answer 13):** scenario 23 passes in exact-cash mode,
abstract mode, and `personal_mode = off`, and across large IC-time changes; scenario 24's retainer
payroll is **deferred** with dependents (§1.1) — E5 substitutes "an organization account pays a
character an obligation" (already E4) plus the abstract employer relationship carrying visibility and
risk without posting.

### 1.1 Explicitly out of E5

- **Dependents**: retainers, ghouls, Herd payroll or loyalty/feeding mechanics (answer 13 — Herd is
  not an economic dependent; the Retainers Background has no NPC linkage and a ghoul's domitor is
  free text, `ruleset-wod/src/splats/ghoul.rs`). E5 reserves an extension point only
  (`SupportSource::Dependent` is a variant with no behavior).
- **Routing `home`** (answer 10): `go_home` is a raw teleport (`mush-core/src/commands/movement.rs:139-183`)
  that already consults `check_move_attempt`, so tenancy fails closed through the hook without
  route work.
- **A global economy-disable switch** (answer 14): organization accounts and the economy
  infrastructure stay available regardless of the WoD personal mode.
- **Automatic trait changes**: the economy never writes a sheet dot and never files trait-loss jobs
  because a scheduled process ran (answer 4). Permanent Resources changes go through the existing
  `+xp/spend` approval path (`ruleset-wod/src/xp.rs:112-300`).
- **A `RulesValue` refactor** (answer 16): the K worktree already has `mush_core::RulesValue` (sim and
  tactics migrated; org still local); E5 uses the core type after the K rebase and touches nothing
  else.
- **Housing reading ruleset sheets** (answer 17): `mush-housing` knows tenancies, properties, access,
  and support assessments — never `backgrounds:Resources`. `ruleset-wod` interprets dots and submits
  explicit plans.
- **Haven mechanics** (wards, security, escape routes — answer 15): Haven dots describe exceptional
  qualities, recorded as a ruleset-side `HavenQualities` note on the profile; no mechanics in E5.
- **Markets, quotes, catalogs, fences** (E6/E8); **player `@dig`** through tenancy (answer 5).

---

## 2. Cut structure

| Cut | Crate(s) | One line | Verify axis |
|-----|----------|----------|-------------|
| **E5a** | `mush-housing` (pure) | `Property`/`Tenancy`/`AccessTerms`/`SupportPlan` records, the tenancy state machine (never auto-ends), the support assessment, the access predicate, the story-window rules | fail-open access, state-machine holes |
| **E5b** | `mush-housing` module | persistence + boot, the `MoveAttempt`/`CarryAttempt` hooks, cadence-driven assessment, the exact-mode bridge to E4 obligations, the story window (job + optional scene + bounded re-notify + level-2 escalation), `@property/*`, `@tenancy/*`, `+home`, `+guest` | offline-home safety, notice bounds, hook bypasses |
| **E5c** | `ruleset-wod` | `[wod.economy]` config, `SpendingProfile`/`CareerProfile`, sources and stances, the period evaluation on the cadence clock, prompts with the attention budget and non-response defaults, front attribution, chargen residence step and mode-dependent approval hook, `+career`, `+prompt` | tedium, budget, attribution, mode isolation |

Dependencies: `mush-housing → {core, org, economy, jobs}` (jobs for the story ticket; `open_job`,
`mush-jobs/src/api.rs:113`); `ruleset-wod → {housing, economy, org, jobs, faction}` — the faction
edge is for `FactionId`/`FrontId` only (`mush-faction/src/sim.rs:32-43`) and is K-adjacent; if K
reshapes fronts, the ids stay. No `mush-core` change. Economy gains **cadence clocks in E4**, not
here (§7).

---

## 3. E5a — `mush-housing`, the pure library

### 3.1 Records

```rust
pub struct Property {
    pub id: PropertyId,
    pub name: SmolStr,
    pub anchor: ObjId,                       // the primary room; the EconomicAsset title record lives on it
    pub rooms: BTreeSet<ObjId>,              // ≤ MAX_ROOMS_PER_PROPERTY (16); staff/builder-created ordinary rooms
    pub tier: PropertyTier,                  // 0..=5: the residence's physical class and upkeep, NOT haven quality
    pub landlord: PrincipalId,               // Organization(..) (a named landlord org) or System(wod.housing)
    pub landlord_scope: Option<OrgScopeId>,  // the org compartment that manages it (D-E3-5)
    pub upkeep: Option<Amount>,              // exact-mode rent per cadence period; None in abstract mode
    pub features: Vec<Feature { key: SmolStr, approved_by: ObjId }>,   // approved customizations (answer 5)
    pub status: PropertyStatus,              // Vacant | Let | Withdrawn
    pub revision: u64,
}
pub struct Tenancy {
    pub id: TenancyId,
    pub property: PropertyId,
    pub tenants: BTreeSet<ObjId>,            // ≤ MAX_TENANTS (8)
    pub guests: BTreeSet<ObjId>,             // tenant-managed, ≤ MAX_GUESTS (16)
    pub terms: AccessTerms { inspection: Option<InspectionTerm { who: PrincipalId, when: Always | WhileDelinquent }> },   // explicit only (answer 9)
    pub funding: Funding,                    // Abstract { plan: Option<SupportPlan> } | Exact { contract: ContractId, obligation: ObligationId }
    pub cadence: CadenceId,                  // the campaign clock this tenancy is assessed on (answer 7)
    pub home_anchor: Option<ObjId>,          // set on Active for each tenant; NEVER cleared automatically (answer 8)
    pub status: TenancyStatus,               // Pending | Active | Grace { since } | Delinquent { since } | EvictionPending { window: StoryWindow } | Ended { how: Resolution }
    pub history: Vec<TenancyEvent>,          // bounded (32)
    pub revision: u64,
}
pub struct StoryWindow { pub opened: i64, pub job: Option<u64>, pub scene: Option<SceneId>, pub notified: u8, pub escalation: Option<EscalationId> }
pub enum Resolution { TenantLeft, StaffEnded { by, reason }, Waived { by }, Settled }
/// What the ruleset asserts about a resident's means, per cadence period.
pub struct SupportPlan {
    pub resident: ObjId,
    pub level: SupportLevel,                 // 0..=5 — derived by the ruleset, opaque to housing
    pub sources: Vec<SupportSource>,         // Career{..} | Patron | Inheritance | Criminal | Domain | Dependent(reserved) — for display/attribution only
    pub asserted_at: i64,
    pub revision: u64,
}
```

### 3.2 The state machine (D-E5-1: a home never disappears on its own)

```text
Pending ── landlord + tenant accept ──▶ Active
Active ── period Unsupported / rent missed ──▶ Grace{since}         (one private notice)
Grace ── supported again / paid ──▶ Active                          (silent)
Grace ── GRACE_PERIODS elapsed ──▶ Delinquent{since}                (one private notice; landlord notice)
Delinquent ── supported again / arrears paid ──▶ Active             (silent)
Delinquent ── DELINQUENT_PERIODS elapsed ──▶ EvictionPending{window} (one private notice; job opened; optional scene)
EvictionPending ── supported again / paid ──▶ Active                (the window closes; job resolved "recovered")
EvictionPending ── explicit resolution ──▶ Ended{how}               (tenant `+home/leave`, staff `@tenancy/resolve end|waive`, contract settlement)
EvictionPending ── window elapses ──▶ EvictionPending               (bounded re-notify; level-2 escalation with the recommended command; NO state change)
```

**Only an explicit act ends a tenancy** and clears `home_anchor` / revokes access: a tenant's own
move-out, a staff resolution after a scene, or a deliberate staff waiver. The story window is
*bounded* in notices, not in duration — an offline player's home persists indefinitely (answer 8;
decision 25's ladder handles the paging).

### 3.3 Assessment and access (pure)

```rust
/// Abstract funding: compare the resident's asserted support with the property's tier.
pub fn assess(tier: PropertyTier, plan: Option<&SupportPlan>) -> Assessment;   // Supported | Strained | Unsupported | Unknown(no plan → Unsupported after UNKNOWN_GRACE periods, never instantly)
/// The hook predicate: may `who` enter (or take from) a room of `property` under `tenancy`?
pub fn may_enter(t: &Tenancy, terms: &AccessTerms, who: ObjId, is_staff: bool, now: i64) -> Access;   // Tenant | Guest | Inspection | Staff | Refused
```

Rules: tenants and guests always (regardless of status — a delinquent tenant still enters their
home); `Inspection` only under an explicit term and, for `WhileDelinquent`, only in
`Delinquent`/`EvictionPending` — **legal authority, economic title, and physical access remain
separate** (answer 9); staff bypass; everyone else refused with one neutral text. Carry mirrors
enter: an actor who may not enter may not take from inside.

### 3.4 E5a tests (~25)

Every transition above and every forbidden one (no path from any state to `Ended` without a
`Resolution`); a window elapsing changes nothing but the notify count; recovery from each failure
state; `assess` across tier/level combinations incl. `Unknown`; `may_enter` for tenant, guest,
inspection-always, inspection-while-delinquent (refused when Active), staff, stranger; caps.

---

## 4. E5b — the housing module

- **Persistence:** `property:{id}`, `tenancy:{id}` container docs (corrupt → fail boot); boot
  reconciles: anchor/rooms missing or `GOING` → property `Withdrawn`, tenancies untouched but
  flagged (a home whose rooms vanished is a staff escalation, never an auto-end); a tenant object
  missing → removed from `tenants` with a history event; an exact tenancy whose obligation is
  missing → `EvictionPending` is **not** entered — the tenancy is flagged and escalated. Allocators
  + meta high-water (the E3 fold rule).
- **Title:** creating a property mints an `EconomicAsset` title for `anchor` to the landlord
  principal through the E4-era title-issuing path (the D-E3-13 principal `system:wod.housing`,
  registered by ruleset-wod at boot; staff `@asset/title` remains). `Obj.owner` of the rooms stays
  the builder — never the org (`@chown` requires a player, `building.rs:631-657`; §9.1).
- **Access enforcement:** an `EventHook` on `HookPoint::MoveAttempt` / `CarryAttempt`
  (`mush-core/src/module.rs:305-326`, fired from `movement.rs:98,154`, `building.rs:149,527`,
  `items.rs:55,104,160`) — first veto wins; the hook answers only for rooms in a `Let` property.
  `home` therefore fails closed for an evicted (Ended) resident without touching routing.
- **Assessment tick:** housing subscribes to the E4 **cadence clock** (§7) named by each tenancy:
  when the clock's period advances, assess every abstract tenancy on it (bounded per fire,
  `MAX_ASSESSMENTS_PER_FIRE`; remainder next fire), applying §3.2 with **one private notice per
  transition** and none on `Supported`. Exact tenancies instead mirror their E4 obligation's
  `ObligationChanged` events (`Grace`/`Delinquent` from the obligation; `EvictionPending` after
  `DELINQUENT_PERIODS` on the same cadence).
- **Story window:** entering `EvictionPending` opens a job in queue `wod-story` (`open_job`) titled
  for the ST with the tenancy summary and the recommended commands (`@tenancy/resolve <id>=extend`,
  `@tenancy/resolve <id>=end`, `@tenancy/resolve <id>=waive`), optionally starts a scene
  (`Ctx::start_scene`, `module.rs:1964-1969`) when the ST asks via the job; re-notifies the tenant at
  `WINDOW_RENOTIFY` (bounded to `MAX_WINDOW_NOTICES`, then daily); after `WINDOW_ESCALATE` opens a
  level-2 escalation (08c §4.6) carrying the same commands. Nothing else happens.
- **Notices for offline players:** a `Connected` hook renders pending tenancy notices on reconnect
  (the `mush-mail` pattern, `mush-mail/src/lib.rs:157-167`; `mush_mail::deliver` is crate-private,
  so housing keeps its own bounded pending-notice list per tenant).
- **Commands:** `@property/create <name>=<anchor room>[,rooms…] tier=<n> landlord=<org|housing> [upkeep=]`,
  `@property/rooms|tier|feature|withdraw`, `@tenancy/create <property>=<tenant…> [cadence=] [inspect=<who>:<always|delinquent>]`,
  `@tenancy/resolve <id>=extend|end|waive`, `@tenancy/show|list`; player `+home` (residence, status,
  guests, the pending notice), `+home/leave` (explicit move-out → `Ended{TenantLeft}`), `+guest
  add|remove <who>`. Refusals render through the economy's existence-hiding discipline (a property
  the actor may not view is "No such property.").

Tests (~25): scenario 23's housing half in abstract mode — supported periods are silent; an
`Unsupported` plan walks Grace → Delinquent → EvictionPending with exactly one notice each and no
movement; the window elapsing twice re-notifies and escalates but the tenant still enters and
`home` still works; `@tenancy/resolve end` clears the anchor and the hook then refuses; support
restored at any stage returns to Active silently; a stranger, a guest, and an inspection-term
holder against each status; the offline tenant's reconnect notice; exact mode via a real E4
obligation with the same transitions; reboot at every state; boot with a vanished room flags and
escalates rather than ending; large cadence jumps assess once with bounded catch-up.

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

`off` creates no WoD economic profile; the platform economy (org accounts, title, vaults) is
untouched. In `abstract` no personal `wod.usd` account exists or is shown. In `exact`
ruleset-wod registers `wod.usd` (scale 2) and the faucet/sink principals `system:wod.income`,
`system:wod.housing`, `system:wod.chargen`; the Resources-to-income table (config
`[wod.economy.income]`, one amount per dot) is authoritative **only** in this mode (answer 3).

### 5.2 Profiles, sources, stances

```rust
pub struct SpendingProfile {                 // ruleset-wod's, per character (`wod:profile:{obj}`)
    pub resources: u8,                       // read from backgrounds:Resources at evaluation, cached for display
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
the per-faction pool), emit the front-attributed evidence for `Exploit`, and **submit a
`SupportPlan` to `mush-housing`** for each tenancy the character holds (answer 17 — the ruleset
interprets, housing assesses). In `exact` mode the evaluation additionally maintains the income
obligation (faucet or employer account → personal account) at the configured amount; in
`abstract` nothing posts (answers 3, 12).

### 5.3 Attention budget, non-response, attribution (answers 18–20)

- **One unresolved prompt per character.** `Prompt { id, kind: Opportunity{front} | HousingPressure | StrainRelief | Advancement, options: Vec<PromptOption>, default: PromptDefault::StatusQuo | PendingWindow, opened_at, expires_after_periods }`. A second prompt while one is open is **not** queued (it is dropped with a history note; the cheapest honest behavior — a queue is a homework list). Housing's own notices are notices, not prompts; the housing *prompt* (downsize / seek support / wait) occupies the slot.
- **Per-faction off-screen pool.** Opportunities attributed to a faction's front count against
  `MAX_OFFSCREEN_OPS_PER_FACTION` (8) across all characters; when the pool is full no new
  front-attributed opportunity is generated for that faction. E5 counts its own; the pool is the
  reserved seam for the K-series faction simulation to share.
- **Non-response.** When a prompt expires, its `default` applies: `StatusQuo` (nothing changes; a
  one-line history note) or `PendingWindow` (the story window stays open for the ST). Expiry never
  adds strain, never lowers support, never notifies as a failure.
- **Front attribution.** Every opportunity, every `Exploit` evidence event, and every housing story
  job carries a `FrontRef` when it concerns a faction; canon events use `kind = "wod.career.exposure"
  | "wod.housing.pressure"` with `source: "front:{faction}:{front}"` and the operation id in
  `detail`. A generic `scene.concluded` with any faction member is **never** an attribution (08c §6.2
  is amended to match on `source`, not kind + entities).

### 5.4 Chargen and approval (answer 15)

ruleset-wod's chargen approval handler (the existing payload path, `ruleset-wod/src/lib.rs:386-391`
pattern) creates, per `personal_mode`: nothing (`off`); a `SpendingProfile` from Resources
(`abstract`); a personal `wod.usd` account plus configured starting funds minted from
`system:wod.chargen` (`exact`). A new chargen stage **Residence** lists vacant staff-built
properties whose tier the derived support level can carry and lets the player choose one (or none);
approval creates the tenancy (`Pending → Active`) through housing. Haven dots never select or
discount a property; they populate `HavenQualities`.

### 5.5 Commands

`+career` (view; `/stance <coast|advance|exploit|conceal>`; `/engage on|off`; `/source add|remove …`),
`+prompt` (view the one pending prompt; `+prompt <option>`), `+resources` (abstract: support level,
strain, sources; exact: balance via the economy's `@account/show` rules). Staff: `@wod/economy`
(show effective config), `@career/set <who>=…` (employer, front).

Tests (~30): scenario 23 end to end in abstract mode (Resources 3 quietly supports a tier-2
residence for N periods with no output; drop Resources via a staff `@sheet/set` → the next period's
plan is `Unsupported` → Grace → Delinquent → EvictionPending with one notice each, a job opened,
no movement, `home` still works; `@tenancy/resolve end` after a staff scene ends it); the same in
exact mode with a real obligation; `off` creates nothing and shows nothing; every stance's table
row; `Exploit` emits one front-attributed canon event per period and `Conceal` none; the attention
budget drops a second prompt; expiry applies `StatusQuo` with no strain; the per-faction pool caps
opportunities; a cadence jump evaluates once; the economy never writes the sheet (assert
`sheet:wod` unchanged across every test); chargen creates the right artifact per mode; the
Residence step offers only tier-appropriate vacancies.

---

## 6. Build discipline

As 08c §7: per-cut probe-confirmed adversarial verify before the next cut; clippy `-D warnings` and
the workspace tests; commit per cut; push on the owner's word. Re-ground §4's E4 references against
the built E4 before E5b starts.

## 7. Amendments this plan feeds back into E4 (08c Draft 0.4)

1. **Cadence clocks ship in E4**, not "registered clocks later": `EconomicClock::Cadence(CadenceId)`
   with economy-owned `cadence:{id}` records `{ id, name, source: RealUtc | WorldIc | Manual,
   period, catch_up_cap, position, advanced_at }`, advanced by the economy sweep from the source
   clock (bounded; excess recorded as forgiven periods) or by `@econ/cadence advance` for `Manual`;
   `EconomicSchedule` may name a cadence, in which case `next_due` is a period index. Owner answer 7
   ("a named campaign/domain clock with a configurable cadence and bounded catch-up") is this.
2. **Canon attestation matches a specific source**, not kind + entities: `Attestation { by: Canon
   { kind, source } }` — owner answer 20.
3. *(post-K)* Doc 09 C6 gives events a `CausalRef { root: CommitToken, parent, ordinal }` and
   `SemanticEvent { kind, schema, truth, eligible_witnesses }`. E5's `FrontRef` and its
   `wod.career.*`/`wod.housing.*` canon kinds are the pre-C6 attribution shape; when C6 lands, the
   front/operation ids ride `SemanticEvent.truth` and the commit token is the source, so front
   attribution nests inside the kernel's causal chain rather than running as a parallel system —
   no change to the front-attribution rule itself. In K's code today only the action-local
   `PreparedCausalRef` exists (`mush-core/src/action.rs:2265-2731`); `FrontRef` over the shipped
   `FactionId`/`FrontId` is the right pre-C6 shape. E5's period evaluation runs on an E4 cadence
   clock, which is doc 09's `ClockRef::Registered { owner: economy, .. }` (08c §12).

## 8. Decisions

- **D-E5-1 A tenancy never ends automatically.** Automatic processing stops at `EvictionPending`
  with a bounded story window; ending, clearing the home anchor, and revoking access are explicit
  acts (answer 8). Duration-unbounded, notice-bounded, escalated per decision 25.
- **D-E5-2 Personal capacity is abstract by default; the income table is exact-mode-only**
  (answers 1–3). A `SupportLevel` (0..=5), not a balance, is what housing assesses.
- **D-E5-3 The economy never writes a sheet dot and never files trait-loss jobs** (answer 4).
  Delinquency produces strain, status, and a story hook.
- **D-E5-4 Access is enforced only through the movement/carry hooks; landlords have no implied
  entry** (answers 9–10). Inspection is an explicit term.
- **D-E5-5 `mush-housing` is game-neutral and sheet-blind; `ruleset-wod` interprets dots and
  submits `SupportPlan`s** (answer 17).
- **D-E5-6 A career is one source among several; stances alter pressure and opportunity, never
  income** (answer 11). No automatic dot gain from `Advance`.
- **D-E5-7 Haven dots are qualities, not housing tier or rent** (answer 15).
- **D-E5-8 Attention budget of one, drop-not-queue; non-response is the status quo; every progress
  event names a front** (answers 18–20).
- **D-E5-9 Config is WoD-owned (`[wod.economy]`)**; no platform-wide economy switch (answer 14).
- **D-E5-10 Dependents are deferred; `SupportSource::Dependent` is a reserved variant with no
  behavior** (answer 13).
- **D-E5-11 Assessment and evaluation run on an E4 cadence clock**, never on an IC:OOC ratio or a
  literal calendar (answer 7).

## 9. Constants

`MAX_ROOMS_PER_PROPERTY` 16 · `MAX_TENANTS` 8 · `MAX_GUESTS` 16 · `GRACE_PERIODS` 1 ·
`DELINQUENT_PERIODS` 2 · `UNKNOWN_GRACE` 1 · `WINDOW_RENOTIFY` 1 period · `MAX_WINDOW_NOTICES` 3, then
daily · `WINDOW_ESCALATE` 2 periods · `MAX_ASSESSMENTS_PER_FIRE` 128 · `STRAIN_MAX` 10 ·
`COAST_OPPORTUNITY_PERIODS` 6 · `MAX_OFFSCREEN_OPS_PER_FACTION` 8 · prompt default expiry 2 periods ·
history caps 32.

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

## 11. Open questions (defaults chosen)

1. `GRACE_PERIODS` 1 and `DELINQUENT_PERIODS` 2 on the cadence — i.e. three assessed periods from
   the first unsupported one to the story window.
2. Whether a second prompt while one is open is dropped (default) or replaces the open one when it
   is more urgent (housing pressure over an opportunity).
3. Whether the chargen Residence step is mandatory when `housing = true` (default: optional; a
   character may start without a residence).
