# Economy, organizations, ownership, and accountable value

**Status:** Draft 0.2 — the reference design. **E0, E1, and E2 are SHIPPED** (`mush-org` and
`mush-economy` now exist); see [08a-economy-build-status.md](08a-economy-build-status.md) for the
build ledger, the decisions made while building, and the E3 pickup point — read it alongside §18.
Reconciled with the shipped tactical spine (F3 control channels/lease, M32 `TacticalUnit`/`UnitMutationPlan` + the `asset` seam, M33 tactics module)<br>
**Scope:** cross-ruleset economic accounts and journals, first-class organizations, scoped
authorization and delegation, asset title and custody, obligations and contracts, background
careers and housing, fencing and markets, and tactical production/logistics<br>
**Applies to:** `mush-core`, the `mush-org` and `mush-economy` foundation crates (shipped),
`mush-faction`, `mush-canon`, `mush-sim`, rulesets, and presentation clients<br>
**Companions:** `docs/design/06-object-behaviors-spatial-domains.md` and
`docs/design/07-npc-control-and-tactical-ai.md`

This document defines a cross-game economy whose subject is **accountable value**, not merely
money. World of Darkness, Shadowrun, and TacSim present very different economic games, but all need
the same trustworthy substrate: principals that can own value, scoped accounts and asset stores,
authorized transfers, recurring obligations, contracts, provenance, and an immutable audit trail.
Rulesets decide which currencies and abstractions exist, what goods cost, how income is earned, and
how much detail is fun. The foundation owns conservation, authorization, durability, and privacy.

The motivating vertical slices are deliberately different:

- A World of Darkness character's background career quietly supports housing, retainers, vehicles,
  and ordinary expenses until a missed obligation becomes a story hook.
- A Shadowrun employer escrows a run payment, the team divides it, runners buy equipment and
  upgrades, and a fence prices stolen goods using provenance, liquidity, delay, and heat.
- A TacSim organization allocates a divisional budget, reserves materials and factory capacity,
  produces or repairs a faction-owned unit, stores it in an authorized facility, and assigns it to
  an operator without transferring title.

The system is inspired by the durable lessons of mature organization-heavy games: organizations
are owners in their own right; accounts and asset stores are compartmentalized; roles grant
specific, scoped powers rather than vague omnipotence; delegation is explicit; and consequential
actions are attributable. This is an independent RustMUSH design. No third-party implementation,
data format, UI, command syntax, formulas, or source structure is part of this specification.

---

## 1. Goals

1. Provide one exact, durable transaction engine for every ruleset without imposing one currency,
   price model, or level of economic prominence.
2. Make characters, NPCs, organizations, and registered system authorities first-class economic
   principals.
3. Generalize factions' useful organizational properties without making every organization a
   political faction.
4. Separate cosmetic titles from semantic, scope-limited authorization.
5. Distinguish asset title, physical custody, assigned operation, and maintenance responsibility.
6. Support exact currencies and commodities alongside ruleset-owned abstract wealth, credit, and
   entitlement systems.
7. Model holds, escrow, obligations, contracts, and production reservations without blocking the
   single-writer world loop or relying on an LLM.
8. Preserve an immutable, double-entry economic journal and atomically maintain rebuildable account
   projections.
9. Publish only visibility-filtered economic evidence to canon and NPC knowledge; exact journal
   truth never becomes public merely because it was recorded.
10. Make ordinary expenses automatic and exception-driven so the economy creates decisions and
    stories rather than chores.
11. Let physical trade, delivery, housing, production, and transport use real spatial paths, access
    checks, observation triggers, and interception opportunities.
12. Give staff bounded audit, recovery, faucet/sink, and inflation diagnostics.

## 2. Non-goals

- One universal currency, exchange rate, auction house, or global market.
- A stock exchange, banking simulator, tax code, or complete supply-chain model in the first
  vertical slices.
- Turning World of Darkness into a mandatory accounting game.
- Assigning an exact cash price to every favor, influence rating, relationship, or narrative asset.
- Treating an item's baseline catalog cost as its permanent market value.
- Using floating-point arithmetic for authoritative amounts.
- Allowing rulesets, scripts, NPCs, or LLM output to mutate balances directly.
- Treating ordinary MUSH `Obj.owner`, `Obj.location`, or writable attributes as sufficient economic
  title or custody records.
- Duplicating physical spatial truth inside the economy module.
- Giving every organization member access to every organization account or asset.
- Deleting or editing posted transactions. Corrections are new compensating transactions.
- Simulating every meal, cartridge, kilometer, or minor expense unless a ruleset deliberately opts
  into that granularity.
- Charging arbitrary accumulated upkeep because the server was down or IC time was advanced by
  staff.

---

## 3. The common model

| Foundation concept | World of Darkness | Shadowrun | TacSim |
|---|---|---|---|
| Income contract | career, patron, domain income | run payment, salary, fencing | budget allocation, supply contract |
| Recurring obligation | housing, retainers, lifestyle | lifestyle, debt, subscriptions | payroll, maintenance, storage |
| Asset | haven, vehicle, weapon | gear, cyberware, vehicle | unit, facility, materiel |
| Organization account | coterie, clan office, business | corporation, gang, runner team | company, regiment, logistics division |
| Restricted custody | shared haven or ghoul equipment | team stash or issued gear | assigned unit or divisional hangar |
| Production/service | influence and contractors | repair, clinics, fabrication | construction, refit, repair |
| Economic evidence | leases, payroll, bribery | payment trails, stolen goods | requisitions, manifests, production logs |

The commonality is not that every game has dollars. It is that value moves between accountable
principals under policy, creates or satisfies obligations, changes claims over assets, and leaves
evidence. A ruleset may make that loop central, occasional, or nearly invisible.

### 3.1 Vocabulary

- A **principal** can own accounts or asset title and can be the party to a contract.
- An **organization** is a durable principal with members, scopes, roles, grants, and delegations.
- An **instrument** is one kind of fungible value: currency, commodity, credit token, or other exact
  quantity.
- An **account** holds exactly one instrument for exactly one principal.
- A **transaction** is an immutable set of balanced journal lines.
- A **hold** reserves available value without yet transferring it.
- An **asset** is a non-fungible world object with economic title and optional encumbrances.
- A **vault** applies custody and access policy to a real container, interior, or facility.
- An **obligation** schedules a possible settlement such as rent, payroll, or maintenance.
- A **contract** binds parties, typed conditions, consideration, escrow, and settlement.
- A **work order** reserves inputs and capacity to deliver a service or produce/modify assets.
- An **economic observation** is visibility-filtered evidence derived from an action; it is not the
  authoritative financial record.

---

## 4. Architectural boundaries

```text
mush-core
    ObjId, World single writer, Ledger storage primitive,
    WriteBatch atomicity, scheduler, events, query/command seams
       |
       +-- mush-org
       |      organization identity, membership, roles, capabilities,
       |      scopes, policy, delegation, organization audit
       |
       +-- mush-economy
       |      instruments, accounts, journal, holds, asset title,
       |      vault bindings, obligations, contracts, work orders
       |
       +-- mush-faction
       |      political identity, relationships, territory, fronts,
       |      standings, epistemic visibility; backed by an organization
       |
       +-- mush-canon / NPC knowledge
       |      observer-visible consequences and evidence, never balances
       |
       +-- mush-sim / housing / rulesets
              physical custody and routes, access, catalogs, prices,
              careers, markets, fencing, production recipes, presentation
```

### 4.1 `mush-core`

Core should gain only universally necessary seams:

- typed monotonic IDs or owner-neutral references required at module boundaries;
- the existing chunked `Ledger` as a persistence primitive;
- the existing same-command `WriteBatch` boundary for object and module writes;
- a small registered clock-provider reference if obligations need clocks other than world IC time;
- world events or synchronous query seams needed to observe committed economic changes.

Core does not know what nuyen, rent, a corporation, a fence, a hangar, a career, or a factory is.
It must not acquire a global `money` field on `Obj`.

### 4.2 `mush-org` (shipped, E1)

`mush-org` owns organization identity and authorization:

- monotonic, never-reused `OrgId`;
- organization records and lifecycle;
- membership ground truth and membership visibility;
- cosmetic titles;
- reusable role templates;
- semantic capabilities and exact scopes;
- direct grants, time-bounded delegations, limits, and approval policies;
- compartments to which accounts and vaults bind;
- bounded live indexes and an authoritative organization-action ledger.

This is a promotion/generalization of existing faction membership ideas, not a parallel membership
system. **As shipped (E1c), `mush-faction` does NOT consume org membership — it keeps its own
membership/level model and exposes standing to org policies through the `RegisteredPredicate` seam
(`FactionStandingProvider`); a membership merge was judged redundant and dropped.** A backing-`OrgId`
mapping is deferred to E3. See [08a](08a-economy-build-status.md).

### 4.3 `mush-economy` (E2 shipped; holds/assets/contracts are E3/E4)

`mush-economy` owns exact accountable value:

- instrument registration;
- accounts and materialized balances;
- the double-entry economic journal;
- idempotency records;
- holds and escrow;
- asset title, assignment, and encumbrances;
- vault records bound to physical containers;
- obligations, contracts, and work orders;
- transaction planning, authorization, posting, reversal, boot validation, and audit queries;
- faucet/sink and flow telemetry.

It depends on `mush-core` and `mush-org`. It consumes faction standing only through a narrow query
provider; it does not depend on faction simulation, canon, spatial, or any ruleset.

### 4.4 `mush-faction`

`mush-faction` remains the political and epistemic layer:

- factions, relationships, standings, territory, fronts, clocks, and strategic resources;
- hidden/covert political knowledge;
- the mapping from a faction to its backing `OrgId`;
- policy inputs such as “allied organizations may dock” or “hostile principals cannot trade.”

Not every organization is a faction. A household, runner team, shell company, service provider,
apartment cooperative, or temporary task force may own accounts and assets without holding
territory or participating in faction standings.

The existing `Faction.resources` and bounded `ResourceChange` tail are transitional. Exact
currencies and commodities migrate to economy accounts and the economic journal. Strategic,
non-fungible ratings such as “influence.police” may remain faction/ruleset state when they are not
conserved transferable quantities.

### 4.5 Rulesets and feature modules

Rulesets and higher modules own semantics and presentation:

- instrument names, symbols, scales, and availability;
- catalog entries and baseline costs;
- exact versus abstract wealth policy;
- careers, lifestyle tiers, housing consequences, and grace;
- contract templates, fence pricing, markets, fees, taxes, and credit;
- recipes, facility capabilities, production time, maintenance, salvage, and depreciation;
- commands, panels, prose, and ruleset-appropriate disclosure.

They submit typed plans to `mush-economy`. They never edit account projections or append journal
entries themselves.

Opaque ruleset-extension blobs (`ruleset_ext`, `driver_data`) use the shipped schema-tagged
`RulesValue { schema, version, data }` pattern — as M28 poses, M31 `ModeRules`, and M32 unit rules
do — never a bare `serde_json::Value`, so a ruleset can version and migrate its own payload.

---

## 5. Principals and organizations

### 5.1 Economic principals

```rust
// As shipped (E1a, crates/mush-org/src/ids.rs): PrincipalId is NOT Copy (it holds a
// non-Copy SystemPrincipalId), and SystemPrincipalId is a validating canonical newtype.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrincipalId {
    Object(ObjId),
    Organization(OrgId),
    System(SystemPrincipalId),
}

// Private field; built via a canonicalizing `new()` (trim + lowercase over
// [a-z0-9_.-], serde try_from), the ChannelId pattern. Registered by a module at
// boot; an arbitrary string from a command is never accepted as one.
pub struct SystemPrincipalId(SmolStr);
```

`Object` covers characters and NPCs and permits staff-approved object principals where a ruleset
needs an estate or trust. `Organization` is the normal shared owner. `System` represents explicit
faucets, sinks, escrow authorities, and clearing accounts. A system principal is registered at boot
by a module; an arbitrary string from a command is never accepted as one.

An absent or destroyed object principal freezes its accounts and contracts for recovery; it does
not erase them. Character destruction therefore cannot destroy money or launder title. Rulesets
decide inheritance, estate, unclaimed-property, and final write-off policy.

**As shipped (E1a), `PrincipalId::Object` is deliberately *clean* — no embedded generation.** The
"reused id" premise turned out false: `ObjId` is never recycled (a destroyed object leaves a
permanent tombstone), so a recycled id cannot resurrect authority in the first place. A generation
guard would live in a *side-table* (the F3 `control_gen` shape) only if some future binding must
outlive its object — never as part of the id's identity/equality/hash.

### 5.2 Organization record

```rust
pub struct Organization {
    pub id: OrgId,
    pub name: SmolStr,
    pub aliases: Vec<SmolStr>,
    pub status: OrgStatus,
    pub hidden: bool,
    pub charter: String,
    pub root_scope: OrgScopeId,
    pub revision: u64,
    pub ruleset_ext: RulesValue, // schema-tagged {schema, version, data}
}

pub enum OrgStatus {
    Active,
    Suspended,
    Dissolving,
    Closed,
}
```

Closing an organization is a workflow, not a delete. It must resolve or transfer accounts, assets,
holds, obligations, contracts, memberships, and delegations. IDs are never reused.

### 5.3 Membership, titles, and role templates

```rust
pub struct OrgMembership {
    pub org: OrgId,
    pub member: ObjId,
    pub status: MembershipStatus,
    pub titles: Vec<SmolStr>,       // presentation only
    pub roles: BTreeSet<RoleId>,    // templates, not final authority
    pub visibility: Visibility,
    pub since: i64,
    pub revision: u64,
}

pub struct OrgRole {
    pub id: RoleId,
    pub org: OrgId,
    pub name: SmolStr,
    pub grants: Vec<GrantTemplate>,
    pub revision: u64,
}
```

“Treasurer,” “Quartermaster,” “Sheriff,” and “Lance Commander” are ruleset-facing names. The
authorization engine never checks those strings. It resolves the grants attached to the role plus
any direct or delegated grants.

### 5.4 Scopes, capabilities, and compartments

```rust
pub struct OrgScope {
    pub id: OrgScopeId,
    pub org: OrgId,
    pub parent: Option<OrgScopeId>,
    pub name: SmolStr,
    pub status: ScopeStatus,
}

pub struct CapabilityGrant {
    pub id: GrantId,
    pub org: OrgId,
    pub grantee: ObjId,
    pub capability: CapabilityId,
    pub scope: ScopeSelector,
    pub limit: Option<GrantLimit>,
    pub delegable: bool,
    pub valid_from: Option<i64>,
    pub valid_until: Option<i64>,
    pub revision: u64,
}
```

Example capabilities include:

```text
economy.account.view
economy.account.transfer
economy.account.approve
economy.audit.read
asset.title.transfer
asset.assign
vault.view
vault.deposit
vault.withdraw
org.role.manage
org.grant.delegate
production.order.create
production.order.approve
```

A scope selects an exact account, vault, work order, or bounded organization subtree. Scope nesting
is capped (recommended maximum depth: 8). Authorization is default-deny; explicit deny wins over
allow; no grant silently expands because a new child scope was created. A role can grant access to
“current descendants” only through an explicit selector whose behavior is visible to administrators.

Do not hard-code “seven wallet divisions” or numbered hangars. Model named scopes and bind accounts
and vaults to them.

### 5.5 Policies, access lists, and standings

Capabilities say what an actor is generally empowered to do. Resource policy decides whether that
capability applies to this account, vault, asset, facility, or operation:

```rust
pub struct AccessPolicy {
    pub id: PolicyId,
    pub owner: PrincipalId,
    pub rules: Vec<PolicyRule>,
    pub default: PolicyEffect,
    pub revision: u64,
}

pub struct PolicyRule {
    pub effect: PolicyEffect,
    pub subject: PolicySubject,
    pub capability: CapabilityId,
    pub scope: ScopeSelector,
    pub conditions: Vec<PolicyCondition>,
}

pub enum PolicySubject {
    Principal(PrincipalId),
    Organization(OrgId),
    OrganizationRole { org: OrgId, role: RoleId },
    OrganizationMember(OrgId),
    RegisteredPredicate {
        provider: SmolStr,
        predicate: SmolStr,
        arguments: serde_json::Value,
    },
}

pub enum PolicyEffect {
    Allow,
    Deny,
}
```

The registered-predicate seam admits rules such as faction standing, licensing, employment,
clearance, tenancy, or doctrine without making `mush-org` depend on those modules. For example,
`mush-faction` may register “standing at least Friendly toward organization X.” Evaluation returns
only allow/deny and a staff-safe reason code; it must not reveal a hidden faction, membership, or
standing value to the caller.

Explicit access-list entries and denies take precedence over broad organization/standing rules.
Every policy evaluation is revision-stamped so a deferred withdrawal, purchase, or LLM-proposed
action becomes stale when the ACL, standing, membership, or source grant changes.

### 5.6 Delegation

```rust
pub struct Delegation {
    pub id: DelegationId,
    pub grantor: ObjId,
    pub grantee: ObjId,
    pub source_grant: GrantId,
    pub capability: CapabilityId,
    pub scope: ScopeSelector,
    pub limit: Option<GrantLimit>,
    pub expires_at: i64,
    pub source_revision: u64,
    pub status: DelegationStatus,
}
```

A delegation may only narrow its source grant. It cannot last beyond the source grant, increase an
amount limit, add a scope, or become delegable unless the source explicitly permits subdelegation.
**As shipped (E1a), a delegation is re-validated by FULLY re-resolving its live source grant on every
authorization** — existence, current validity window, still-`delegable`, capability match, scope
containment in the source's *current* scope, and that the grantor still holds the source
(`src.grantee == grantor`). Revocation, expiry, reassignment, and scope-narrowing therefore all take
effect immediately. `source_revision` is only a fast staleness *hint*, NOT the invalidation
mechanism (an expired source bumps no revision, so equality alone would miss it). Delegation depth is
bounded; E1 allows one hop.

High-value actions may use an approval policy rather than one broad grant:

```rust
pub enum ApprovalPolicy {
    OneAuthorizedActor,
    Threshold { approvals: u8, from_role: Option<RoleId> },
    NamedSet { approvals: u8, actors: BTreeSet<ObjId> },
}
```

Approvals bind to an immutable plan hash and expire. Changing amount, destination, asset set, or
terms invalidates them. **As shipped, E1 ships only the `ApprovalPolicy` *type*; the
approval-collection workflow (binding approvals to the plan hash) lands with the economy (E2/E3),
which has plans to approve.**

### 5.7 Faction migration

> **As shipped (E1c), this was scoped down — read this note over the paragraph below.** E1c ships a
> `FactionStandingProvider` (an `OrgPredicateProvider`): org access policies read *live* faction
> `member`/`officer`/`intel` standing through the `RegisteredPredicate` seam. **The faction keeps its
> own membership + level/officer/intel model** (R-D1) — there is NO membership merge, and mirroring
> faction membership into `OrgMembership` was **dropped** as redundant with the predicate seam. The
> **Faction→backing-`OrgId` mapping and the `Faction.resources` migration are deferred to E3**, where
> account ownership defines what that mapping must carry. See [08a](08a-economy-build-status.md) §5.
>
> *(Original E1 intent, retained for context:)*
No DATA migration is needed — dev1 is wipeable, so there is no legacy membership store to preserve.
But the shipped M21–M25 `mush-faction` CODE is refactored to consume `mush-org`: rework faction
membership to BE organization membership (a faction references its backing `OrgId` and keeps NO second
membership store), while faction-specific visibility, politics, intel thresholds, territory, and
relationships stay in `mush-faction`. This is the largest change to shipped surface in this doc, so
sequence it deliberately — stand up `mush-org` (E1) first, then rework faction onto it and delete its
own membership answer once authorization/disclosure tests pass. Store the `Faction -> OrgId` mapping
explicitly even if the two allocators happen to align.

---

## 6. Instruments, accounts, and exact amounts

### 6.1 Instrument definitions

```rust
pub struct InstrumentDef {
    pub id: InstrumentId,
    pub ruleset: SmolStr,
    pub code: SmolStr,              // e.g. "nuyen", "usd", "tacsim.steel"
    pub kind: InstrumentKind,
    pub scale: u8,                  // decimal minor-unit scale, 0 for indivisible
    pub transferable: bool,
    pub negative_policy: NegativePolicy,
    pub status: InstrumentStatus,
    pub display: InstrumentDisplay,
    pub revision: u64,
}

pub enum InstrumentKind {
    Currency,
    Commodity,
    Requisition,
    Entitlement,
}
```

Instrument codes are namespaced and registered by trusted modules. Authoritative values are signed
`i64` minor units. Arithmetic uses checked operations; validation may sum in `i128`, but a posted
line and projected balance must fit `i64`. Saturating arithmetic is forbidden. Wire protocols carry
minor units as decimal strings plus a formatted display string so JavaScript clients never lose
precision.

No implicit exchange exists between instruments. An exchange is an ordinary multi-instrument trade:
each instrument balances independently across its own lines.

### 6.2 Accounts

```rust
pub struct Account {
    pub id: AccountId,
    pub owner: PrincipalId,
    pub instrument: InstrumentId,
    pub scope: Option<OrgScopeId>,
    pub name: SmolStr,
    pub status: AccountStatus,
    pub posted_balance: i64,
    pub held_amount: i64,
    pub credit: CreditPolicy,
    pub policy: PolicyRef,
    pub last_journal_seq: u64,
    pub revision: u64,
}

pub enum AccountStatus {
    Active,
    Frozen,
    Closing,
    Closed,
}
```

Each account holds one instrument. `posted_balance` and `held_amount` are materialized projections,
not competing truths. Available funds are derived:

```text
available = posted_balance - active holds
```

A negative posted balance is legal only when the account's explicit, ruleset-validated credit
policy permits it. Closing an account requires zero balance, no active holds, and no live obligation
or contract references, or an atomic transfer to a successor account.

### 6.3 Exact and abstract wealth

World of Darkness-style Resources must not be implemented as pretend dollars that decrement on
every coffee. Abstract wealth is a ruleset policy layered above exact accounts:

```rust
pub struct SpendingProfile {
    pub owner: PrincipalId,
    pub rating: u8,
    pub supported_lifestyle: SmolStr,
    pub liquid_account: Option<AccountId>,
    pub strain: u8,
    pub revision: u64,
    pub ruleset_ext: RulesValue, // schema-tagged {schema, version, data}
}
```

The ruleset answers whether a purchase is routine, temporarily straining, financing-required, or
impossible. A routine purchase may post no exact personal transaction; a consequential purchase may
create debt, reduce liquid reserves, add strain, or require a scene. The engine treats that result
as a typed authorized plan. The spending rating itself is not freely transferable and is not
double-entry currency.

Shadowrun and TacSim may use exact accounts extensively. Deployments may still choose abstraction
for incidental ammunition, fuel, or meals while keeping major purchases exact.

---

## 7. The economic journal

> **As shipped (E2 — read this over §7.2/§7.5 below).** The journal is the sole authority: balances,
> the idempotency index, the reversed set, the txid index, and the allocators are all **re-derived
> from the journal at boot** — the idempotency record is NOT a separately-persisted boot authority (no
> `idempotency:*` doc), so a companion-doc drop cannot double-post. E0's retained-write table closed
> the old `Ledger` seal-drop window (§7.2's hazard is resolved). Boot is **full-journal-replay** with
> **directional self-heal**: a projection that merely lags re-derives silently to the head; only real
> drift, an ahead projection (lost tail), an unreplayable journal, or a per-instrument conservation
> break freezes the *whole* economy (single latch — per-account isolation is later). The durability
> **checkpoint and cold-chunk paging are deferred** (full-replay is the E2 baseline). See
> [08a](08a-economy-build-status.md).

### 7.1 Transaction model

```rust
pub struct EconomicTransaction {
    pub id: TransactionId,
    pub journal_seq: u64,
    pub idempotency: IdempotencyKey,
    pub cause: EconomicCause,
    pub actor: PrincipalId,
    pub posted_at: EconomicTimestamp,
    pub description: String,
    pub lines: Vec<JournalLine>,
    pub reverses: Option<TransactionId>,
    pub visibility: EconomicVisibility,
    pub plan_hash: [u8; 32],
}

pub struct JournalLine {
    pub account: AccountId,
    pub instrument: InstrumentId,
    pub delta: i64,
    pub memo: SmolStr,
}

pub enum EconomicCause {
    Command { serial: u64 },
    Internal { kind: SmolStr, id: u64 },
    External(CausalRef),
    Staff { reason: String },
    Ruleset { namespace: SmolStr, correlation: SmolStr },
}

pub struct CausalRef {
    pub provider: SmolStr, // e.g. "scene", "canon", "sim", "jobs"
    pub kind: SmolStr,
    pub id: SmolStr,
    pub revision: Option<u64>,
}
```

Internal cause kinds cover contracts, obligations, and work orders without widening the enum for
every new economy record. Registered external references correlate a transaction with a scene,
canon event, simulation operation, or job while keeping `mush-economy` independent of those crates.
The provider validates the reference when a plan requires it; the journal preserves the opaque
attribution even if the external record is later archived.

Normative invariants:

1. A transaction has at least two non-zero lines.
2. Every referenced account and instrument is active and the account's instrument matches the line.
3. For each instrument independently, line deltas sum to exactly zero.
4. Every projected balance and available-balance result uses checked arithmetic.
5. Holds, credit limits, authorization, approval policy, and asset preconditions validate against
   one immutable pre-state.
6. `TransactionId`, journal sequence, and idempotency key are unique and monotonic where applicable.
7. A posted transaction is immutable. A reversal is a new transaction with inverse lines and an
   explicit reference.
8. “Minting” and “burning” still balance against registered system source/sink accounts.
9. No success output is released before the durable write acknowledgement.

### 7.2 Relationship to `mush_core::Ledger`

The economy owns a dedicated logical log such as `mod:economy:log:journal:*` using the existing
chunked `mush_core::Ledger` machinery. It does **not** put financial transactions into the canon
ledger, a scene log, or a capped Redis stream.

One post queues, in the same `WriteBatch`:

- the next immutable economic-journal entry;
- every changed account projection;
- changed holds, contracts, obligations, assets, or work orders;
- the idempotency record and allocator meta;
- any world-object or spatial changes that are part of the same validated exchange;
- visibility-filtered canon/evidence events produced from the committed result.

The current `Ledger` documents a narrow non-self-healing failure when a deterministic store
rejection drops the seal-and-roll-over write after RAM has advanced. That is unacceptable for
financial truth. Before the first real economy post, core must land ack-gated ledger rollover (and
the store-read/paging seam needed to restore and audit cold history), or an equivalently strong
transaction-document implementation. Economy is not allowed to waive this prerequisite.

### 7.3 Idempotency and asynchronous callers

```rust
pub struct IdempotencyKey {
    pub namespace: SmolStr,
    pub key: SmolStr,
}

pub struct IdempotencyRecord {
    pub idempotency: IdempotencyKey,
    pub plan_hash: [u8; 32],
    pub transaction: TransactionId,
    pub posted_at: i64,
}
```

Repeating the same key with the same plan returns the original result. Reusing it with a different
plan is an error and an audit event. Keys are mandatory for network callbacks, deferred work, LLM
tool calls, scheduled obligations, contract settlement, and production completion.

### 7.4 Transaction planning

Economy follows the same plan-before-apply rule as the shipped mush-sim `Transition` and M32
`UnitMutationPlan` (`expected_revisions` generalizes F3's lease generation and M32's `expected_revision`):

```rust
pub struct EconomyPlan {
    pub idempotency: IdempotencyKey,
    pub actor: PrincipalId,
    pub requested_lines: Vec<RequestedLine>,
    pub hold_ops: Vec<HoldOp>,
    pub asset_ops: Vec<AssetOp>,
    pub contract_ops: Vec<ContractOp>,
    pub world_ops: Vec<WorldOp>,
    pub cause: EconomicCause,
    pub visibility: EconomicVisibility,
    pub expected_revisions: RevisionSet,
}

pub enum PlanResult {
    Post(ValidatedEconomyPlan),
    NeedsApproval(ApprovalRequestId),
    Refused(EconomyError),
    AlreadyPosted(TransactionId),
}
```

Validation is pure over one world-task snapshot. Apply performs no fallible policy decisions. Any
async quote, LLM proposal, route result, or approval re-enters with expected revisions and is stale
if price, account, policy, ownership, contract, or quote generation changed.

### 7.5 Projections, restart, and audit

The journal is authoritative; account documents are fast materialized projections. Boot must:

1. validate allocator and instrument containers;
2. restore the journal tail using the hardened Ledger seam;
3. load account projections and verify their last sequence/revision;
4. replay journal entries after the latest validated checkpoint;
5. compare replayed and stored balances;
6. freeze the affected account or the whole economy on mismatch—never silently choose a value;
7. rebuild derived owner, scope, hold, contract, and asset indexes;
8. publish no control or spending authority until validation completes.

Periodic checkpoints contain balances, active holds, allocator positions, the covered journal
sequence, and a deterministic state hash. They accelerate boot but do not replace journal history.
Cold audit pages through Ledger chunks; immediate account views may use a bounded derived recent
tail. A derived tail may be rebuilt and must never become authority.

---

## 8. Holds, escrow, obligations, and contracts

### 8.1 Holds

```rust
pub struct Hold {
    pub id: HoldId,
    pub account: AccountId,
    pub amount: i64,
    pub beneficiary: Option<PrincipalId>,
    pub purpose: HoldPurpose,
    pub status: HoldStatus,
    pub expires: Option<EconomicDeadline>,
    pub revision: u64,
}

pub enum HoldStatus {
    Active,
    Captured { transaction: TransactionId },
    Released,
    Expired,
}
```

Creating a hold reduces available funds but does not change posted balance. Capture posts a
transaction and closes the hold atomically. Release and expiry never move money. Holds are the
foundation for run escrow, market orders, hotel/rental deposits, bids, pending approvals, and
production input reservations.

### 8.2 Clock bases

```rust
pub enum EconomicClock {
    WorldIc,
    RealUtc, // administrative/service uses only unless ruleset opts in explicitly
    Registered { provider: SmolStr, id: u64 },
    Manual,
}

pub struct EconomicSchedule {
    pub clock: EconomicClock,
    pub next_due: i64,
    pub cadence: Option<i64>,
    pub maximum_catch_up: u8,
}
```

Every obligation declares its clock. A deployment must not accidentally charge a month of rent
because the server was offline, nor charge years of upkeep because staff jumped IC time. Scheduled
processing is bounded per world step, idempotent, and subject to a maximum catch-up count. Excess
periods become an explicit staff-visible reconciliation state, not an unbounded loop.

### 8.3 Obligations

```rust
pub struct Obligation {
    pub id: ObligationId,
    pub debtor: PrincipalId,
    pub creditor: PrincipalId,
    pub debit_account: AccountId,
    pub credit_account: AccountId,
    pub amount: ObligationAmount,
    pub schedule: EconomicSchedule,
    pub grace: GracePolicy,
    pub settlement: SettlementPolicy,
    pub status: ObligationStatus,
    pub cause: ObligationCause,
    pub revision: u64,
}

pub enum ObligationStatus {
    Active,
    Grace { missed: u8, deadline: i64 },
    Delinquent,
    Suspended,
    Satisfied,
    Cancelled,
}
```

An obligation attempts settlement; it does not assume settlement succeeded. Failure advances a
ruleset-owned typed consequence policy. Rent should normally move through grace, delinquency,
service restriction, and a story-visible eviction process—not instantly teleport a character or
delete a home.

### 8.4 Contracts

```rust
pub struct EconomicContract {
    pub id: ContractId,
    pub issuer: PrincipalId,
    pub parties: BTreeSet<PrincipalId>,
    pub terms: ContractTerms,
    pub conditions: Vec<TypedCondition>,
    pub escrow: Vec<HoldId>,
    pub split: Vec<PayoutShare>,
    pub deadline: Option<EconomicDeadline>,
    pub state: ContractState,
    pub visibility: EconomicVisibility,
    pub revision: u64,
}

pub enum ContractState {
    Draft,
    Offered,
    Accepted,
    Active,
    ReadyToSettle,
    Settled { transaction: TransactionId },
    Disputed,
    Cancelled,
    Expired,
}
```

Settlement conditions consume typed, attributable facts—scene outcomes, deliveries, approved jobs,
or ruleset events. They do not parse narrative prose or accept “the LLM said the run succeeded.”
Contract negotiation may be narrative; settlement is deterministic.

Team splits use explicit shares or exact amounts whose rounding policy is part of the accepted
terms. Remainders go to a named account, never whichever participant happened to iterate first.

---

## 9. Assets, title, custody, assignment, and vaults

### 9.1 Asset interests

```rust
pub struct EconomicAsset {
    pub object: ObjId,
    pub title_owner: PrincipalId,
    pub assignment: Option<AssetAssignment>,
    pub maintenance_account: Option<AccountId>,
    pub catalog: Option<CatalogRef>,
    pub encumbrances: Vec<Encumbrance>,
    pub acquisition: Option<AcquisitionRecord>,
    pub status: AssetStatus,
    pub revision: u64,
}

pub struct AssetAssignment {
    pub assigned_to: PrincipalId,
    pub purpose: SmolStr,
    pub valid_until: Option<i64>,
    pub policy: PolicyRef,
}

pub enum Encumbrance {
    Lease { lessor: PrincipalId, contract: ContractId },
    Lien { creditor: PrincipalId, amount: i64, instrument: InstrumentId },
    Collateral { contract: ContractId },
    Disputed { claimant: PrincipalId, reason: String },
}
```

This enables:

- corporation owns; employee operates; project account maintains;
- character owns; faction vault stores; another character temporarily borrows;
- lessor owns; tenant possesses; contract governs access;
- player physically steals an object but does not magically acquire title;
- unit is captured, disputed, salvaged, reassigned, or written off with an attributable history.

`Obj.owner` remains MUSH control/administration and is not economic title. Changing one must not
silently change the other.

Granularity follows the tactical model: a composed unit (M32 `TacticalUnit`) is one titled asset, but
each installed component whose `asset: Option<ObjId>` points at a tradeable object gets its OWN
`EconomicAsset` title record — so a faction-owned mech can carry a leased autocannon with a distinct
owner and encumbrance. The `asset` scrub M33 deliberately deferred (nulling a component's pointer when
its asset object dies) is an economy-cut responsibility, handled by the section 15.3 destruction gates
and world-event backstop.

### 9.2 Physical custody is not duplicated

Physical location remains authoritative where it already belongs:

- ordinary `Obj.location` for ordinary MUSH containment;
- `SpatialParent` and its projection for simulated/interior/attached placement;
- a housing/property subsystem for tenancy and residence;
- a vault binding for policy around a real container or interior.

Economy records title and contractual assignment. It queries physical custody through registered
providers. A title transfer never teleports an asset. A physical move never transfers title unless
a validated transaction explicitly includes both operations.

### 9.3 Vaults

```rust
pub struct Vault {
    pub id: VaultId,
    pub owner: PrincipalId,
    pub scope: Option<OrgScopeId>,
    pub name: SmolStr,
    pub backing: CustodyRef,
    pub policy: PolicyRef,
    pub status: VaultStatus,
    pub revision: u64,
}

pub enum CustodyRef {
    Container(ObjId),
    SpatialInterior(ObjId),
    Registered { provider: SmolStr, id: u64 },
}
```

A vault does not maintain a second peer list of contents. Contents derive from the backing
container/provider. Deposit and withdrawal are atomic world/spatial transitions authorized by the
vault policy. Access to one division's vault conveys nothing about another division's vault or
wallet.

Destroyed, dangling, or structurally invalid backing locations freeze the vault and trigger
recovery. They never cause the economy module to delete its title records.

### 9.4 Catalogs, prices, and provenance

Rulesets register catalog definitions:

```rust
pub struct CatalogEntry {
    pub id: CatalogId,
    pub ruleset: SmolStr,
    pub code: SmolStr,
    pub tags: BTreeSet<SmolStr>,
    pub baseline: Option<QuotedAmount>,
    pub availability: serde_json::Value,
    pub revision: u64,
}
```

Baseline cost is a reference for chargen, staff tools, NPC valuation, or quote generation. It is not
an intrinsic permanent price. Actual transactions use immutable quotes whose revision, seller,
buyer, location, expiry, and terms are captured in the plan.

Provenance is append-only evidence attached to acquisition, title changes, theft reports, serial
identifiers, and known custody events. A fence evaluates provenance through the buyer's available
knowledge; the pricing engine does not reveal hidden truth to the seller.

---

## 10. Work orders, production, and services

### 10.1 Generic work order

```rust
pub struct WorkOrder {
    pub id: WorkOrderId,
    pub owner: PrincipalId,
    pub requester: PrincipalId,
    pub driver: SmolStr,
    pub recipe: SmolStr,
    pub facility: FacilityRef,
    pub inputs: Vec<ResourceReservation>,
    pub capacity: Vec<CapacityReservation>,
    pub outputs: Vec<PlannedOutput>,
    pub payment: Vec<HoldId>,
    pub schedule: EconomicSchedule,
    pub destination: Option<VaultId>,
    pub state: WorkOrderState,
    pub revision: u64,
    pub driver_data: RulesValue, // schema-tagged {schema, version, data}
}

pub enum WorkOrderState {
    Draft,
    AwaitingApproval,
    Reserved,
    InProgress,
    Blocked,
    Ready,
    Delivered,
    Cancelled,
    Failed,
}
```

`mush-economy` owns reservations, authorization, lifecycle, payment, and title delivery. A registered
driver owns recipe validation, facility capability, duration, output construction, repair/refit
rules, and interruption consequences.

Capacity is a reservation, not necessarily a transferable account balance. Materials may be
commodity accounts or asset objects. Completion revalidates the work-order revision and facility;
it never runs expensive simulation on the scheduler callback.

### 10.2 TacSim production

A TacSim production order normally:

1. resolves faction doctrine and divisional authority;
2. reserves currency/requisition and exact material inputs;
3. reserves a compatible facility slot;
4. records the intended title owner, destination vault, and maintenance account;
5. advances on a bounded campaign/production clock;
6. creates or modifies the unit through the ruleset driver;
7. transfers the output into faction title and physical custody;
8. releases unused reservations and posts actual cost;
9. journals who requested, approved, built, received, and assigned it.

Operating costs should be modeled only at decision-relevant granularity. Fuel, ammunition, parts,
maintenance, storage, and payroll may be exact, batched per sortie/period, or abstracted by doctrine.
The ruleset chooses; the journal remains exact for whatever it elects to post.

### 10.3 Mobility and other services

The `MobilityJob` proposed in doc 07 becomes an economic service contract, built on the shipped M30
`Journey` route primitive and the F3 control lease:

- dispatch accepts only after payment/credit authorization;
- the vehicle and driver remain real assets with real availability;
- route, pickup, embark, travel, and drop-off remain `mush-sim` operations;
- cancellation, waiting, danger premiums, damage, and failure settle according to typed terms;
- payment does not grant the passenger a movement or weapon control lease;
- route and payment knowledge remain separately authorized.

The same structure covers couriers, tow trucks, clinics, repair crews, ghoul retainers, and
mercenary transport.

---

## 11. Ruleset vertical slices

### 11.1 World of Darkness: housing and background careers

Housing should be its own feature module layered on organization, economy, and spatial/access
systems:

```rust
pub struct Tenancy {
    pub id: TenancyId,
    pub property: ObjId,
    pub landlord: PrincipalId,
    pub tenants: BTreeSet<PrincipalId>,
    pub contract: ContractId,
    pub obligation: ObligationId,
    pub access_policy: PolicyRef,
    pub home_anchor: Option<ObjId>,
    pub status: TenancyStatus,
    pub revision: u64,
}

pub enum TenancyStatus {
    Pending,
    Active,
    Grace,
    Delinquent,
    EvictionPending,
    Ended,
}
```

Tenancy grants scoped access and a valid home/residence relationship. It does not bypass ordinary
travel: going home still follows traceable routes and observation/interception triggers.

A background career is an income and pressure policy, not a repetitive “work” command:

```rust
pub struct CareerProfile {
    pub actor: ObjId,
    pub employer: Option<PrincipalId>,
    pub kind: SmolStr,
    pub income: IncomePolicy,
    pub cadence: EconomicSchedule,
    pub time_demand: u8,
    pub visibility: u8,
    pub risk: u8,
    pub stance: CareerStance,
    pub status: CareerStatus,
    pub strain: u8,
    pub revision: u64,
    pub ruleset_ext: RulesValue, // schema-tagged {schema, version, data}
}

pub enum CareerStance {
    Coast,
    Advance,
    Exploit,
    Conceal,
}
```

Routine settlement is automatic. A player who wants the mini-game can change stance, cultivate
contacts, take risks, negotiate, or play scenes. Everyone else sees compact period summaries and
only receives prompts when income, exposure, strain, or employment changes materially.

Recommended housing failure progression:

```text
payment succeeds
    -> no interruption
payment fails
    -> grace + private notice
grace expires
    -> delinquent + debt/strain + landlord action
continued failure
    -> service/access restrictions allowed by ruleset
    -> eviction-pending story window
    -> explicit move/tenancy transition, never silent teleport
```

Retainers and ghouls may have payroll obligations, allowances, equipment assignments, or loyalty
consequences. The system should make paying them possible and attributable, not mandatory busywork
for chronicles that abstract it.

### 11.2 Shadowrun: run, pay, upkeep, gear, and fences

The first Shadowrun slice:

1. A Johnson/employer principal offers a typed run contract.
2. Funds or credit are placed on hold in escrow.
3. Team members accept an explicit split and expense policy.
4. Scene/job outcomes satisfy typed contract conditions.
5. Settlement posts once, idempotently, with deterministic rounding.
6. Lifestyle obligations settle automatically.
7. Major gear, vehicle, cyberware, clinic, and repair purchases use exact quotes.
8. Team/corporate gear distinguishes title, vault custody, and assigned operator.

A fence is a service/market policy, not a generic “sell for percentage” command:

```rust
pub struct FenceQuote {
    pub id: QuoteId,
    pub fence: PrincipalId,
    pub seller: PrincipalId,
    pub assets: Vec<ObjId>,
    pub offer: QuotedAmount,
    pub settlement_delay: EconomicDeadline,
    pub heat: u8,
    pub confidence: u8,
    pub required_delivery: Option<VaultId>,
    pub expires_at: i64,
    pub observed_revisions: RevisionSet,
}
```

Pricing may consider known provenance, demand, legality, relationship, faction standing, liquidity,
delivery risk, and delay. Accepting the quote reserves payment; settlement requires actual delivery
through ordinary custody/spatial transitions. Hidden provenance can later create confiscation,
blackmail, investigation, or reputation consequences without retroactively changing the posted
sale.

### 11.3 TacSim: divisional budgets, logistics, and production

The first TacSim slice:

1. A faction-backed organization has operations, logistics, and production scopes.
2. Each scope has separate accounts and vaults.
3. A quartermaster can inspect and reserve logistics resources but cannot pay payroll.
4. A production officer creates an order; a threshold policy approves high-value construction.
5. Inputs and capacity reserve atomically.
6. Completion produces a faction-titled unit in the correct facility/vault.
7. A commander assigns the unit and an operating budget to a pilot without transferring title.
8. Deployment, ammunition, repairs, salvage, capture, loss, and write-off remain attributable.

Later campaign economics may add local supply, transport manifests, blockades, insurance,
depreciation, and player markets. Those build on the same accounts, assets, routes, and work orders;
they are not required for the foundation.

---

## 12. Canon, epistemics, privacy, and audit

### 12.1 Three distinct questions

For every economic fact, keep these separate:

1. **What is true?** The economic journal, account projection, contract, and title records.
2. **Who is authorized to query exact truth?** Organization grants, account/vault policy, staff
   capability, and audit scope.
3. **What evidence exists in the fiction, and who observed it?** Canon and NPC knowledge events
   produced from payments, meetings, deliveries, manifests, thefts, or compromised systems.

Recording an exact transaction must not make it public. Conversely, someone may observe a delivery
without knowing the price or legal owner.

### 12.2 Economic visibility

```rust
pub struct EconomicVisibility {
    pub parties: bool,
    pub account_auditors: bool,
    pub organization_scope: Option<OrgScopeId>,
    pub public_summary: Option<DisclosureLevel>,
    pub evidence_policy: EvidencePolicyId,
}

pub enum DisclosureLevel {
    Existence,
    ApproximateValue,
    ExactValue,
    FullTerms,
}
```

Rulesets generate observer-relative output. Error messages, autocomplete, lists, balances, panels,
metrics, and audit tools must not distinguish “secret account exists but denied” from “no such
account” for unauthorized actors.

### 12.3 Audit records

The economic transaction journal already attributes transfers. `mush-org` separately journals:

- organization, scope, role, policy, and grant changes;
- delegations, revocations, approvals, and asset assignments;
- vault withdrawals and deposits;
- freezes, recovery actions, and staff overrides.

Rejected attempts are security telemetry, not authoritative economic history. They are bounded and
rate-limited so an attacker cannot create an unbounded audit queue.

Exact audit access is itself capability-gated and may be logged. A ruleset may make a compromised
audit trail discoverable evidence, but the LLM or NPC memory layer never receives records the
principal could not lawfully or fictionally know.

### 12.4 Economic events

Post-commit typed events should be compact and non-secret by construction:

```rust
pub enum EconomicEvent {
    TransactionPosted { id: TransactionId, cause: EconomicCause },
    HoldChanged { id: HoldId, status: HoldStatus },
    ContractChanged { id: ContractId, state: ContractState },
    ObligationChanged { id: ObligationId, status: ObligationStatus },
    AssetTitleChanged { object: ObjId, revision: u64 },
    WorkOrderChanged { id: WorkOrderId, state: WorkOrderState },
}
```

Consumers query through authorized projections rather than receiving balances or private parties in
the event payload. This follows the existing discipline of exposing a trigger without broadcasting
hidden truth.

---

## 13. LLM and NPC boundary

LLMs may:

- negotiate in character;
- interpret a vague purchasing or destination request;
- produce a contract or quote **draft**;
- explain an account summary already authorized for the actor;
- choose among ruleset-provided legal offers;
- generate narrative reactions to success, delinquency, suspicion, or refusal.

LLMs may not:

- edit balances, holds, title, prices, policy, or contract state directly;
- invent accounts, system principals, assets, or authorization;
- decide that a settlement condition occurred;
- see secret accounts, exact balances, hidden provenance, or audit records outside their principal's
  authorized/epistemic projection;
- post without a deterministic tool call carrying idempotency, plan hash, quote revision, and actor
  capability;
- keep a quote or approval alive after its deadline or revision becomes stale.

Provider failure, latency, malformed output, or budget exhaustion degrades to deterministic menus,
ruleset quotes, or refusal. It never blocks upkeep settlement, production completion, emergency
control, or the world tick.

---

## 14. Faucet, sink, market, and anti-tedium policy

### 14.1 Explicit sources and sinks

Every external creation or destruction of an exact instrument balances against a registered system
account:

```text
off-screen run faucet  system:shadowrun.contracts -> employer/team account
modeled run payment    employer account -> team account
modeled rent payment   tenant account -> landlord account
abstract rent sink     tenant account -> system:wod.housing
production material    logistics account -> system:tacsim.consumed_material
staff correction       named correction account <-> affected account
```

Each system principal declares a category, owning module, allowed instruments, and posting
capability. Telemetry can therefore report sources and sinks by ruleset, cause, principal class,
period, and instrument. “Set balance to 50,000” is not a normal command; staff uses an attributed
balanced correction transaction.

### 14.2 No automatic universal market

Markets are ruleset modules over quotes, holds, contracts, title, custody, and routes. They decide:

- local or global reach;
- listings versus NPC quotes;
- liquidity and spread;
- legality, licensing, taxes, fees, and heat;
- delivery and settlement requirements;
- whether prices are public, approximate, or private;
- whether off-screen supply exists and where its faucet/sink accounts are.

Physical goods should default to local delivery. A remote purchase may create a real shipment or
service journey, preserving observation and interception opportunities.

### 14.3 Tedium guardrails

- Automatic payment is the default for routine obligations.
- Notify on meaningful change or failure, not every successful micro-transaction.
- Aggregate incidental consumption by scene, sortie, day, or lifestyle period.
- Do not reward repeated login clicks or permit an unbounded passive-income loop.
- Background systems progress on declared clocks, not online time.
- Give players stances, budgets, priorities, and exceptions rather than repetitive commands.
- Let deployments disable or abstract subsystems without violating journal invariants.
- Every scheduled processor has bounded work, catch-up, output, and persistence.

---

## 15. Persistence and recovery

### 15.1 Proposed namespaces

`mush-org`:

```text
meta
org:{id}
scope:{id}
member:{org}:{obj}
role:{id}
grant:{id}
delegation:{id}
approval:{id}
log:audit:{chunk}
```

`mush-economy`:

```text
meta
instrument:{id}
account:{id}
asset:{obj}
vault:{id}
hold:{id}
obligation:{id}
contract:{id}
work:{id}
idempotency:{namespace}:{key_hash}
checkpoint:{seq}
log:journal:{chunk}
```

> **As shipped (E2), the economy persists only `meta`, `instrument:{id}`, `account:{id}`, and
> `log:journal:*`.** `idempotency:*` is **not** written — the idempotency index is journal-DERIVED at
> boot (R-B2); `checkpoint:{seq}` is **deferred** (full-replay baseline); and `asset`/`vault`/`hold`/
> `obligation`/`contract`/`work` are **E3/E4** (not yet persisted). `mush-org` similarly ships no
> `approval:{id}` yet (the `ApprovalPolicy` type only).

All IDs are monotonic and never reused. Keys and embedded IDs must agree. Unknown schema versions
do not load active.

### 15.2 Failure policy

Financial corruption must fail closed:

- corrupt allocator, instrument, or journal tail (~~or checkpoint~~): fail economy boot; **(as
  shipped: a contiguity breach, an unknown-account journal reference, or an unparseable doc fails the
  boot)**;
- account projection mismatch: **as shipped, any drift / ahead / conservation break freezes the WHOLE
  economy (a single frozen latch over all accounts)** and requires staff recovery — per-account
  isolation is a later refinement;
- invalid hold/contract/obligation reference: disable and quarantine the record, reserve no value,
  and persist the recovery state;
- missing object principal or asset object: freeze related accounts/title for estate or write-off
  recovery;
- missing organization/scope/policy: deny authorization and freeze dependent shared resources;
- invalid vault backing: freeze withdrawals/deposits but preserve title and contents;
- orphan work order: release nothing until deterministic reconciliation decides whether inputs were
  consumed.

Never skip, delete, saturate, guess, or silently rebuild a balance from one convenient document.
Recovery appends attributed corrective records; it does not rewrite posted history.

### 15.3 Destruction and lifecycle

Object-destruction gates refuse destruction while the object is:

- a live economic asset with unresolved title/encumbrances;
- the backing location of a non-empty or active vault;
- a property with active tenancy;
- a facility with active reservations/work orders;
- an active object principal with unresolved accounts/contracts.

Programmatic destruction is handled by a world-event backstop that freezes affected state and opens
recovery; it never treats destruction as a valid transfer or debt cancellation.

### 15.4 Boundedness

Configuration must cap:

- lines per transaction;
- accounts/assets per plan;
- active holds and obligations per principal;
- organization scopes, hierarchy depth, roles, grants, and delegation depth;
- approvals per request;
- work orders advanced per scheduler step;
- audit query page size and recent-tail size;
- evidence events emitted from one post;
- description, memo, code, and idempotency-key lengths.

Bulk payroll, team splits, or production consumption beyond a transaction cap use a resumable batch
with escrow/staging and an explicit completion state. They do not monopolize one world step.

---

## 16. Commands and presentation

Core module commands are administrative and neutral. **The list below is the design vision; the
verbs/prefixes that actually shipped in E1/E2 are annotated (`✓`) — the rest are later milestones:**

```text
@org/create ✓, @org/list ✓, @org/show ✓, @org/grant ✓, @org/audit ✓   (E1b — staff-only)
@instrument/register ✓, @instrument/list ✓                            (E2b — NOT @economy/instrument)
@account/open ✓, @account/system ✓, @account/show ✓, @account/list ✓   (E2b)
@econ/pay ✓, @econ/mint ✓, @econ/reverse ✓, @econ/audit ✓             (E2b — NOT @economy/*; no /freeze or /validate command)
@asset/*, @vault/*, @obligation/*, @contract/*, @work/*               (E3/E4 — not built)
@org/scope, @org/role, @org/delegate                                  (E1 API via submit(); no dedicated commands yet)
```

Player-facing commands and panels are ruleset-owned:

- WoD may show Resources, lifestyle, housing, career, dependents, and exceptional obligations.
- Shadowrun may show nuyen, run escrow, team funds, lifestyle, debts, gear, quotes, and fences.
- TacSim may show divisional wallets, requisitions, vault manifests, work orders, maintenance, and
  assigned units.

Structured output carries stable IDs, decimal-string minor units, formatted amounts, status, and
authorized actions. The server remains the authority for formatting rules such as currency symbol,
scale, rounding, and approximate disclosure; clients do not infer secret precision.

---

## 17. Testing strategy

### 17.1 Journal and arithmetic

1. Post a two-account transfer and verify per-instrument zero sum, checked balances, journal
   sequence, projections, and one durable batch.
2. Refuse insufficient funds, overflow, mismatched instruments, closed/frozen accounts, malformed
   lines, and an unauthorized actor with no RAM or persistence mutation.
3. Retry an ambiguously acknowledged post and verify exactly one transaction and projection change.
4. Reuse an idempotency key with a different plan and verify refusal plus bounded security audit.
5. Reverse a transaction and verify both immutable records remain and balances compensate.
6. Exchange two instruments and verify each instrument balances independently.
7. Fuzz transaction lines, order, account state, and extreme amounts; no panic, overflow, or
   conservation failure.

### 17.2 Organizations and authorization

8. Give an actor transfer authority over one divisional account and verify sibling accounts and
   vaults remain undiscoverable and inaccessible.
9. Change cosmetic titles without changing authority; change a role grant and verify authority
   changes at its revision boundary.
10. Delegate a narrowed grant, expire/revoke its source, and verify every descendant becomes stale.
11. Require two approvals, mutate the plan, and verify prior approvals cannot authorize it.
12. Hide an organization/membership and verify lists, errors, audit, NPC context, and panels do not
   leak existence.

### 17.3 Holds, obligations, and contracts

13. Create concurrent holds and verify available funds cannot be double-spent.
14. Capture, release, and expire holds idempotently across restart.
15. Settle a recurring obligation, exercise grace/delinquency, and bound catch-up after downtime and
   a large IC-time jump.
16. Complete a contract condition twice and verify payment posts once.
17. Split a payment with deterministic rounding and a named remainder destination.

### 17.4 Assets and custody

18. Transfer title without moving the object; move/steal the object without transferring title.
19. Assign an organization-owned vehicle to an operator and revoke assignment without changing
   title or physical location.
20. Deposit/withdraw through a vault and verify its policy and the spatial/container transition
   commit atomically.
21. Destroy an asset or vault backing programmatically and verify freeze/recovery without lost title
   or phantom contents.
22. Reboot after every title/custody/assignment boundary and reproduce identical state.

### 17.5 Vertical slices

23. Run one WoD career period that quietly covers housing; then force failure through grace,
   delinquency, and an explicit eviction story window without teleportation.
24. Pay a ghoul/retainer from a personal or organization account and verify private attribution and
   optional loyalty consequences.
25. Offer, escrow, complete, split, and settle a Shadowrun job under provider success and failure.
26. Quote and accept a fenced-goods sale; require real delivery and expose only observer-authorized
   provenance/evidence.
27. Reserve, approve, build, vault, and assign a TacSim unit using separate production/logistics
   scopes.
28. Run a mobility-service contract and verify payment does not teleport the vehicle, skip route
   observations, or grant the passenger control authority.
29. Let an LLM propose a purchase/contract, delay its response past quote or policy revision, and
   verify deterministic stale refusal.
30. Rebuild projections from a checkpoint plus journal tail and detect every injected mismatch.

---

## 18. Implementation milestones

These milestones are labeled E0–E8 to avoid colliding with the repository-wide M-numbered delivery
history and docs 06/07's spatial phases.

### E0: durability and boundary prerequisites — **SHIPPED (`565ff1b`, `32ca322`)**

- Land ack-gated `Ledger` seal/rollover. **(Shipped: byte-scan retained-write table re-emits un-acked
  sealed chunks, poison-seal fail-closed, boot contiguity.)** Cold-chunk *paging* / a play-time
  store-read seam was **deferred** — no reader yet; full-journal-replay-from-`data.kv` is the baseline.
- Add authoritative-ledger recovery tests around ambiguous and deterministic store failures.
- Define checked amount/wire types, monotonic economic IDs, idempotency limits, and registered clock
  references. **(The checked `Amount`/wire types actually landed in E2a, not E0.)**
- Prove one plan can atomically queue Ledger, module, object, and cross-module projection writes in
  the existing `WriteBatch` and gate success on durable acknowledgement.

**Exit criterion:** kill/retry/restart tests cannot duplicate or lose a synthetic journal entry,
including exactly at chunk rollover.

### E1: organization identity and scoped authorization — **SHIPPED (`6e9a030`, `11090aa`, `97afebf`)**

- Add `mush-org` records, persistence, boot validation, indexes, inspection, and recovery.
- Add scopes, role templates, capability grants, one-hop delegation, revisions, and policy checks.
- Add organization-action audit and hidden-membership disclosure tests. **Org creation is staff-only.**
- ~~Add faction-to-backing-organization mapping and a non-destructive membership adapter.~~
  **(E1c instead shipped a `FactionStandingProvider` — org policies read live faction
  member/officer/intel standing via the `RegisteredPredicate` seam, the faction level model unchanged.
  The backing-`OrgId` mapping + membership adapter are DEFERRED to E3; membership-mirroring was
  dropped.)**

**Exit criterion:** two organizations can have identically named roles but different scoped powers;
an actor can operate one account/vault scope without learning a sibling exists; **faction
authorization and visibility are unchanged (resolved live via the predicate seam — no data migration).**

### E2: instruments, accounts, journal, and transfers — **SHIPPED (`8bed472`, `547aec6`)**

- Add `mush-economy` instruments, accounts, materialized projections, allocator/meta, and journal.
- Add pure plan validation, posting, reversal, idempotency, durable acknowledgement, boot replay,
  and bounded audit. **The durability *checkpoint* was deferred (R-D1) — full-journal-replay is the
  baseline; idempotency/reversed/txid/allocators are journal-DERIVED at boot. A new STRICT durable
  ack-gate was added to core for money (invariant 9).**
- Add registered system source/sink accounts and flow telemetry. **(E2b records a system-owned
  unbounded-credit account; the rich system-principal metadata + flow telemetry are future.)**
- Add staff-only neutral inspection and recovery commands (`@instrument`/`@account`/`@econ`).

**Exit criterion:** journal tests 1–7 and restart/rebuild tests pass on in-memory and Redis suites;
there is no balance mutation API outside validated posting.

### E3: organization wallets, assets, and vaults — **NEXT**

- Bind organization accounts to scopes and enforce capability/approval policies. **(The `scope` field
  + the `WorldAuth`→`authorize_in_world` scoped path already exist; E3 makes the divisional-wallet
  vertical real and wires `ApprovalPolicy` collection to the E2a `plan_hash`.)**
- **Deferred from E1c:** the Faction→backing-`OrgId` mapping and the non-destructive
  membership/authorization adapter — decide the linkage shape here, driven by account ownership.
- Add title, assignment, maintenance payer, encumbrance, and lifecycle records (the four-axis
  title/custody/**operation (= the shipped F3 control lease)**/maintenance separation, D5).
- Add vaults bound to ordinary containers and `mush-sim` interiors.
- Add atomic payment + title + custody plans and programmatic-destruction recovery.
- Begin migration of exact `Faction.resources` pools into organization accounts.

**Exit criterion:** organization/asset scenarios 8–12 and 18–22 pass, including restart at every
transition boundary.

### E4: holds, escrow, obligations, and contracts

- Add holds, capture/release/expiry, and available-balance projection.
- Add registered clocks, bounded due processing, catch-up policy, obligations, and grace state.
- Add typed contracts, escrow, approvals, deterministic splits, settlement, disputes, and
  cancellation.
- Add compact visibility-safe economic events and canon/evidence projection.

**Exit criterion:** scenarios 13–17 pass with duplicate scheduler events, restart, stale revisions,
large time jumps, and hidden parties.

### E5: World of Darkness housing and career slice

- Add a property/tenancy module with access and home-anchor integration.
- Add ruleset spending profiles, lifestyle policy, careers, stances, strain, and automatic period
  settlement.
- Add optional retainer/ghoul payroll and organization-owned housing/business accounts.
- Keep routine output compact and make failures story-state transitions.

**Exit criterion:** scenarios 23–24 pass with exact-cash mode, abstract-Resources mode, economy
engagement disabled, and large IC-time changes.

### E6: Shadowrun run-and-pay slice

- Add run contract templates, employer escrow, team acceptance/split, expenses, and settlement.
- Add lifestyle/debt obligations and exact major-purchase quotes.
- Add item provenance, fence quotes, delayed settlement, delivery, and heat hooks.
- Add team/corporate vault and issued-gear ownership flows.

**Exit criterion:** scenarios 25–26 pass with provider disabled, slow, malformed, or stale; no model
result can post without current deterministic authorization.

### E7: TacSim requisition and production slice

- Add divisional budget/vault templates and threshold approvals.
- Add generic work-order reservations and a TacSim recipe/facility driver.
- Add unit construction/repair/refit, destination custody, title, maintenance, and assignment.
- Add sortie- or period-level operating-cost aggregation and salvage/capture/write-off paths.

**Exit criterion:** scenario 27 passes through line SSH and one structured panel, including
production interruption, restart, capture, and recovery.

### E8: local markets, services, and economic operations

- Add a generic quote/listing seam, local market providers, fees, delivery contracts, and bounded
  price history.
- Integrate mobility/courier services with real routes and interceptable delivery.
- Add economic dashboards for sources, sinks, velocity, concentration, delinquency, production, and
  suspicious flow.
- Tune caps, indexes, checkpoint cadence, and audit paging from measured workloads.

**Exit criterion:** scenarios 28–30 pass under load; market/service activity remains bounded,
privacy-preserving, and spatially honest.

---

## 19. Decisions and rationale

The following are recommended defaults for this draft.

### D1. The foundation models accountable value, not money

**Decision:** exact money, commodities, requisitions, and entitlements share instruments/accounts;
abstract wealth and influence remain ruleset policies when they are not conserved transferable
quantities.

### D2. Organizations are distinct from factions

**Decision:** add `mush-org` and make factions reference backing organizations. Every faction should
normally have one backing organization; not every organization is a faction. Promote rather than
duplicate membership and authorization.

### D3. The economic journal uses the existing Ledger substrate

**Decision:** one dedicated double-entry economy journal uses hardened `mush_core::Ledger` chunked
documents plus materialized account projections in the same `WriteBatch`. It is correlated with,
but not stored in, canon. Ack-gated rollover was the E0 blocker (**shipped**); cold-chunk paging was
deferred (no reader — full-journal-replay-from-`data.kv` is the E2 baseline).

### D4. Roles are templates; capabilities authorize

**Decision:** titles are cosmetic, roles bundle grants, and final authorization checks a semantic
capability, exact scope, amount/operation limit, time, revision, delegation chain, and approval
policy. Default deny.

### D5. Title, custody, operation, and maintenance are separate

**Decision:** economy owns title/encumbrance/assignment; spatial or ordinary containment owns
physical location; control-station leases (the shipped F3 control-channel/lease bundle) own
operation; an account may own maintenance liability.
No relationship silently implies another.

### D6. Posted transactions are immutable

**Decision:** never edit/delete. Reverse with an attributed compensating transaction. Staff
corrections balance against registered correction accounts.

### D7. Every scheduled obligation names its clock

**Decision:** world IC, registered domain/campaign, real UTC, and manual clocks are explicit.
Real-time billing is opt-in. Catch-up is capped and excess becomes reconciliation, not an unbounded
charge loop.

### D8. Every exact faucet and sink is an account

**Decision:** system principals make value creation/destruction balanced and measurable. Rulesets
cannot call “add money”; they submit a transaction involving an authorized source/sink account.

### D9. Baseline cost is catalog data, not object truth

**Decision:** rulesets own catalogs and quote generation. Transactions capture actual immutable
quotes. Location, availability, relationship, provenance, market conditions, and delivery may all
change actual price.

### D10. Routine economy is exception-driven

**Decision:** automatic settlement, aggregation, and compact summaries are defaults. Player-facing
choices are stances, budgets, contracts, priorities, and exceptions—not repetitive work/payment
commands.

### D11. Markets are local providers, not a foundation singleton

**Decision:** no universal auction house or exchange rate. Market providers layer quotes, holds,
contracts, title, custody, and routes; physical delivery preserves observation and interception.

### D12. LLMs propose; deterministic code posts

**Decision:** model output may create a draft or select from legal options. Posting always
revalidates typed terms, policy, visibility, quote revision, deadline, and idempotency on the world
task. Provider failure never prevents core economic service.

---

## 20. Deferred questions

These do not block E0–E4:

- Whether a later ruleset needs household/estate principals beyond ordinary organizations.
- Whether securities, equity shares, or interest-bearing loans justify specialized instruments.
- Whether tax authorities and insurance become contract templates or separate feature modules.
- Whether high-volume player markets need a dedicated derived Redis index beyond Ledger paging.
- Whether accounting periods and financial statements deserve a generic reporting layer.
- Whether strategic faction influence should ever become transferable, or remain entirely outside
  exact economy.
- How much operating cost TacSim should aggregate per sortie versus track as exact commodities.
- Which economic observations should create automatic NPC salience versus remain queryable evidence.

The implementation should answer these with measured vertical-slice needs rather than widening the
foundation in advance.
