# E3 implementation plan — organization wallets, assets, and vaults

**Status:** Draft 0.3 — reviewed (three passes: adversarial money-safety, code-grounding, design-fit
against docs 08/08a); every confirmed finding is folded below and listed in §11. Draft 0.3 adds the
as-built amendments each cut reported (marked *as built*) and the build outcome in §12. Companion to
[08-economy-organizations-ownership.md](08-economy-organizations-ownership.md) (the design) and
[08a-economy-build-status.md](08a-economy-build-status.md) (the build ledger + the E3 pickup point).
Read 08a §"Pick up here: E3" first; this document turns that pickup into cuts, records, seams,
decisions, and tests, grounded in the code on `main` at `d7f6707`.

Everything named with a `file:line` was verified against the tree by the grounding review;
everything named as "new" does not exist yet. Where this plan departs from doc 08's sketch, the
departure is a numbered decision (D-E3-n, §8).

---

## 1. Scope

E3 delivers, from doc 08 §18-E3 and 08a:

1. **Org wallets bound to scopes + approval enforcement.** Open org-owned accounts under a scope;
   enforce `economy.account.post` (exists) **and** a per-transaction `ApprovalPolicy` bound to the
   E2a `plan_hash` (`ApprovalPolicy` is type-only today, `mush-org/src/record.rs:329-350`).
2. **Assets: title, assignment, maintenance payer, encumbrance, lifecycle** (§9.1) — the four-axis
   title / custody / operation (= the F3 control lease) / maintenance separation (D5).
3. **Vaults** bound to ordinary containers and `mush-sim` interiors (§9.3); deposit/withdraw as
   atomic world/spatial transitions authorized by vault policy.
4. **Atomic payment + title + custody plans** — one validated plan, one durable bundle.
5. **Programmatic-destruction recovery** — the §15.3 gates and the world-event backstop, including
   the M32/M33-deferred `InstalledComponent.asset` scrub (`mush-tactics/src/unit.rs:322-338`,
   `module.rs:147-149`).
6. **Faction → backing-`OrgId` mapping + the exact `Faction.resources` migration** (deferred from
   E1c, 08a decision 5).

**Exit criterion (doc 08):** organization/asset scenarios 8–12 and 18–22 pass, including restart at
every transition boundary.

### 1.1 Explicitly out of E3

- **Two-party consent.** An E3 plan is **single-authority**: every money line and every asset/
  custody op is authorized for the ONE `actor`, live. A sale between two principals needs
  counterparty consent, which is E4's holds/contracts. E3 ships the multi-effect plan *mechanism*;
  E4 adds consent without changing the record shape (D-E3-4). Title transfer needs authority on
  **both** sides (D-E3-12), so in E3 a transfer is staff, an org agent moving between compartments
  they hold, or an org agent taking an org asset into their own name.
- Holds, `held_amount` enforcement, obligations, contracts (`Encumbrance::Lease`/`Collateral` need a
  `ContractId` and are E4 variants of a serde-tagged enum E3 leaves open). An approval **reserves
  nothing** — two approved plans over one account both post until funds run out; the second fails
  cleanly on funds.
- `SpendingProfile` / abstract Resources (E5); catalogs, quotes, provenance beyond an opaque
  `CatalogRef` (E6/E8, `CatalogRef { ruleset, code }` is the pre-registry precursor of §9.4's
  `CatalogId`); the disclosure *filter* (E4) — with one bounded exception, §4.5 reads.
- `PolicyRef` (does not exist anywhere; D-E3-5). A shared player haven is an org: 08a decision 1
  keeps org creation staff-only, so a coterie/team gets a staff-minted org; a non-org vault is
  owner-only + staff in E3.
- The **membership/authorization adapter** deferred from E1c is **struck** (D-E3-11): role
  templates require real `OrgMembership`; a faction-backed org authorizes through direct grants and
  the E1c `FactionStandingProvider` predicates.
- A title-issuing ruleset principal (chargen havens, E7 production output) — shape noted in
  D-E3-13, built by E5/E7. In E3 `Title` (creating a title record) is staff-only.
- Wiring `TacticsModule` into the server (absent from `mush-server/src/modules.rs:70-92` today,
  unrelated to E3). The tactics scrub is built + tested in-crate.
- Per-account isolation of the whole-economy freeze (E2 granularity stays), a `mush-sim` `economy`
  cargo feature (the new `mush-sim → mush-economy` edge is unconditional, §2).
- Evicting an operator when title changes hands: the operation axis is the F3 lease
  (`CommandSubjectLink`/`control_gen`, owned by `mush-sim`), and no relationship implies another
  (D5) — a new title owner has no eviction verb in E3; that is E7's.

---

## 2. Cut structure

E0–E2 established the rhythm **pure library → world-attached module → cross-crate seam**. The
review split the module cut along its two threat models, so five commits in four named cuts:

| Cut | Crate(s) | One line | Verify axis |
|-----|----------|----------|-------------|
| **E3a** | `mush-economy` (pure) | asset/vault/approval records, snapshot growth, `AssetOp`/`CustodyOp`, `validate`/`apply` over invariants 10–20, `plan_hash` v2, `NeedsApproval`, the `CustodyEval` + extended `AuthEval` seams | pure invariants + hash determinism |
| **E3b1** | `mush-economy` module | asset + vault persistence and boot (projection heal / liveness / custody reconciliation), the `CustodyProvider` seam + built-in `ContainerProvider`, `@asset`/`@vault`, the destroy gate + backstop, `@econ/pay` composition | title/custody/money desync |
| **E3b2** | `mush-economy` module + `mush-org` (separate commit) | org-scoped wallets, approvals end-to-end, the system-account debit gate, non-staff reads with existence-hiding, `@org/scope`/`@org/grant` extension, status enforcement, capability vocabulary, `@account/close`/`unfreeze`, `RetireScope` | fail-open authorization |
| **E3c** | `mush-sim`, `mush-tactics` (two commits) | `InteriorCustodyProvider` + interior vaults enabled; tactics `asset` scrub on destroy + boot; operation-axis tests | spatial ↔ economy |
| **E3d** | `mush-faction` | `Faction.org`, `@faction/org`, static migration marking, `@faction/migrate`, view-gated read-through | attribution + idempotent migration |

Dependency directions: `mush-economy → {core, org}` (unchanged); `mush-sim → mush-economy` (**new
edge**, mirrors E1c's "the provider implementation lives with the data it reads" but note `mush-sim`
did not previously sit above economy); `mush-tactics` gains nothing; `mush-faction → {org, economy}`
(new economy dep, `mush-faction/Cargo.toml:13-15` already carries org). The grounding review
confirmed the graph is acyclic including dev-deps (`mush-testkit → {core, proto}` only). `mush-economy`
gains **no** dev-dep on `mush-tactics` — the component-asset test lives in `mush-tactics`.

---

## 3. E3a — the pure library

All in `crates/mush-economy/src/`, world-free, unit-tested on hand-built snapshots like E2a.

### 3.1 Ids (`ids.rs`)

`int_id! { VaultId }`, `int_id! { ApprovalRequestId }` — monotonic, never reused, allocators + meta
resume in E3b. Assets are keyed by `ObjId` (never recycled, `mush-core/src/obj.rs:5-7`).

### 3.2 Records (`record.rs`)

```rust
pub struct EconomicAsset {
    // ---- journal-authority fields (carried in every AssetEffect.post) ----
    pub object: ObjId,
    pub title_owner: PrincipalId,
    /// Org compartment for an org-owned asset — the policy anchor, exactly as `Account.scope`.
    /// Invariant 18: `Some(scope)` ⇒ `title_owner == Organization(scope.org)`.
    pub scope: Option<OrgScopeId>,
    pub assignment: Option<AssetAssignment>,
    pub maintenance_account: Option<AccountId>,
    pub catalog: Option<CatalogRef>,            // { ruleset: SmolStr, code: SmolStr }
    pub encumbrances: Vec<Encumbrance>,         // ≤ MAX_ENCUMBRANCES_PER_ASSET (16)
    pub acquisition: Option<AcquisitionRecord>, // { transaction: TransactionId, at: i64 }
    pub lifecycle: AssetLifecycle,              // Active | WrittenOff  (journaled)
    pub last_journal_seq: u64,
    pub revision: u64,
    // ---- doc-authority latch (NEVER in an effect; the Account `Frozen` split) ----
    #[serde(default)] pub frozen: bool,
    #[serde(default)] pub frozen_reason: Option<SmolStr>,
}
pub struct AssetAssignment { pub assigned_to: PrincipalId, pub purpose: SmolStr, pub valid_until: Option<i64> }
#[serde(tag = "kind")]
pub enum Encumbrance {
    Lien { creditor: PrincipalId, amount: Amount, instrument: InstrumentId },
    Disputed { claimant: PrincipalId, reason: String },   // reason ≤ MAX_REASON_BYTES (512)
    // Lease / Collateral arrive with ContractId in E4.
}
pub enum AssetLifecycle { Active, WrittenOff }

pub struct Vault {
    pub id: VaultId,
    pub owner: PrincipalId,
    pub scope: Option<OrgScopeId>,              // invariant 18 applies
    pub name: SmolStr,
    pub backing: CustodyRef,
    pub status: VaultStatus,                    // Active | Frozen | Closed   (doc-authority)
    pub revision: u64,
}
pub enum CustodyRef { Container(ObjId), SpatialInterior(ObjId), Registered { provider: SmolStr, id: u64 } }

/// Per-account high-value escalation (D-E3-2). `Account` gains
/// `#[serde(default)] approval: Option<ApprovalRule>`. `OneAuthorizedActor` is REFUSED here
/// (it can never gate anything) — clear the rule with `none` instead.
pub struct ApprovalRule { pub threshold: Amount, pub policy: mush_org::ApprovalPolicy }
pub struct ApprovalRequest {
    pub id: ApprovalRequestId,
    pub plan_hash: [u8; 32],
    pub accounts: Vec<AccountId>,               // the accounts whose rules fired
    pub requested_by: PrincipalId,
    pub approvals: Vec<Approval>,               // { approver: ObjId, at: i64 } ≤ MAX_APPROVERS_PER_REQUEST (8)
    pub decisions: Vec<Decision>,               // { actor, at, verdict: Approved|Denied } — the audit trail, same cap
    pub created_at: i64,
    pub expires_at: i64,
    pub status: ApprovalStatus,                 // Pending | Consumed { transaction } | Cancelled | Expired
    pub revision: u64,
}
```

There is deliberately **no `Satisfied` status**: whether a request satisfies its policy is
re-derived live in `validate` from the recorded approvers (D-E3-2, the delegation discipline of
`mush-org/src/authorize.rs:295-340`).

`EconomicTransaction` gains three `#[serde(default)]` fields so every E2 journal entry still
deserializes (E2b boot parses each entry, `boot.rs:74-80`):

```rust
pub asset_effects:     Vec<AssetEffect>,   // { object, kind: AssetEffectKind, post: EconomicAsset (frozen fields zeroed) }
pub custody_effects:   Vec<CustodyEffect>, // { object, vault, direction: Deposit|Withdraw, from: Option<ObjId>, to: ObjId }
pub consumed_approval: Option<ApprovalRequestId>,
```

An `AssetEffect` carries the **full journal-authority post-state**; `frozen`/`frozen_reason` are
always `false`/`None` inside an effect and are ignored by replay and by drift comparison.
`AssetEffectKind` (Created / TitleTransferred / Assigned / Unassigned / Encumbered / Released /
MaintenanceSet / WrittenOff) is for audit and display.

### 3.3 Snapshot (`snapshot.rs`)

`EconomySnapshot` gains `assets: BTreeMap<ObjId, EconomicAsset>`, `vaults: BTreeMap<VaultId, Vault>`,
`approvals: BTreeMap<ApprovalRequestId, ApprovalRequest>`, and derived indexes
`approvals_by_hash: BTreeMap<[u8;32], ApprovalRequestId>` (**one live request per hash**),
`assets_by_owner`, `vaults_by_backing: BTreeMap<CustodyRef, VaultId>` (one vault per backing).
Reads public; `*_mut` `pub(crate)`.

### 3.4 Plan alphabet (`plan.rs`)

```rust
pub enum AssetOp {
    Title         { object: ObjId, owner: PrincipalId, scope: Option<OrgScopeId>, catalog: Option<CatalogRef> },
    TransferTitle { object: ObjId, to: PrincipalId, to_scope: Option<OrgScopeId> },
    Assign        { object: ObjId, to: PrincipalId, purpose: SmolStr, valid_until: Option<i64> },
    Unassign      { object: ObjId },
    Encumber      { object: ObjId, encumbrance: Encumbrance },
    Release       { object: ObjId, encumbrance: Encumbrance },   // exact match
    SetMaintenance{ object: ObjId, account: Option<AccountId> },
    WriteOff      { object: ObjId, reason: String },             // reason ≤ MAX_REASON_BYTES
}
pub enum CustodyOp {
    Deposit  { object: ObjId, vault: VaultId },
    Withdraw { object: ObjId, vault: VaultId, to: ObjId },       // `to` constrained by invariant 15
}
pub struct EconomyPlan {
    /* E2 fields unchanged; EconomyPlan::new keeps its E2 signature */
    pub asset_ops: Vec<AssetOp>,                      // ≤ MAX_ASSET_OPS_PER_PLAN (32)
    pub custody_ops: Vec<CustodyOp>,                  // ≤ MAX_CUSTODY_OPS_PER_PLAN (32); ≤ 1 per object
    pub expected_asset_revisions: Vec<AssetRevision>, // ≤ MAX_ASSET_OPS_PER_PLAN; { object, revision }
}
pub enum PlanResult {
    Post(Box<Prepared>),
    AlreadyPosted(TransactionId),
    NeedsApproval(Box<ApprovalNeed>),                 // { plan_hash, accounts: Vec<AccountId> }
    Refused(EconomyError),
}
```

`Prepared` gains `asset_effects`, `custody_effects`, `consumed_approval`. `PostOutcome` (`api.rs:147-152`)
gains `NeedsApproval(ApprovalRequestId)`; the exhaustive matches at `api.rs:192`, `commands.rs:374`,
and `tests/e2a.rs:301` gain an arm. `expected_revisions` (E2) gains a cap of `MAX_LINES_PER_TX`,
checked in the cheap first block (`engine.rs:89-100`).

New `EconomyError` variants: `NoSuchAsset`, `AssetExists`, `AssetNotActive`, `AssetFrozen`,
`Encumbered`, `NoSuchVault`, `VaultNotActive`, `NotInVault`, `NoCustodyStanding`, `BadDestination`,
`BackingInvalid`, `DuplicateAssetOp`, `DuplicateCustodyOp`, `ScopeOwnerMismatch`,
`ApprovalExpired`, `StaleAsset { object, expected, found }`. (Rendering of these to non-staff is
governed by §4.5's existence-hiding rule.) *As built:* also `NoSuchEncumbrance` (a `Release` naming
a claim the asset does not carry — invariant 12 relies on releases actually clearing),
`AssignmentToOwner` (invariant 13), `HasEffects` (reversal of an effect-carrying transaction),
`ApprovalRequired`, and `Custody(CustodyError)`; `Prepared` also carries `stamp_acquisition: bool`
because `validate` cannot know the txid `apply` will stamp. `ApprovalExpired` is a command-layer
error — `validate` answers an expired request with `NeedsApproval` (the remedy is a new request).

### 3.5 Invariants `validate` proves (extends E2a's nine)

10. A transaction has **≥2 non-zero money lines, or ≥1 asset/custody effect**. When money lines
    exist, invariants 1–4 hold unchanged. Boot replay and the conservation tripwire treat an
    effect-only entry as contributing zero to every instrument.
11. Per object per plan: at most **one title-changing op** (`Title` / `TransferTitle` / `WriteOff`
    are mutually exclusive); `Release`, `Encumber`, `Assign`, `Unassign`, `SetMaintenance` may
    accompany it, and `Release`s apply first. Every referenced asset exists (except `Title`, which
    requires it not exist), is `Active`, and is not `frozen` (`AssetFrozen`).
12. `TransferTitle` and `WriteOff` refuse an asset carrying any encumbrance unless the same plan
    `Release`s every one of them (legal under 11).
13. `Assign` refuses `assigned_to == title_owner`. `TransferTitle` **clears `assignment` and
    `maintenance_account`** in the post-state (both are relationships with the old owner) — the
    effect records it.
14. `SetMaintenance { account: Some(a) }` requires `a.owner == title_owner` **or** `a.scope ==
    asset.scope` (both `Some`), and the actor to be authorized to debit `a` (`AuthEval::authorize`
    with a zero probe delta is not enough — use `authorize_maintenance(actor, account)`, which
    `WorldAuth` implements as "may post from it").
15. Custody: `Deposit` requires the vault `Active`, its backing valid (`CustodyEval::backing_valid`),
    and **custody standing** on the object (`CustodyEval::may_deposit(obj, actor)`: carried —
    `Obj.location == Some(actor)` exactly, never an ancestor walk — **or** co-located with the actor
    and the actor is the title owner / assignee / MUSH controller of the object / staff).
    `Withdraw` requires the vault to hold the object (`CustodyEval::holds`, transitive, depth-capped)
    and `to` ∈ {actor, actor's current location}, live, not `GOING`, and **not the backing of any
    vault** (entering a vault is a `Deposit`). Custody ops never touch title (D5). At most one
    `CustodyOp` per object per plan (`DuplicateCustodyOp`).
16. `expected_asset_revisions` checked like `expected_revisions` (stale → `Refused`).
17. **Approval.** For every account with an `ApprovalRule` whose net debit exceeds `threshold`:
    the snapshot's live request for this `plan_hash` (if any) must be `Pending`, unexpired, cover
    the account, and its recorded approvers — **re-qualified live** via
    `AuthEval::authorize_approver(approver, account, &policy)` (holds `economy.account.approve` on
    the account's scope; `Threshold.from_role` → holds that role via `OrgMembership`; `NamedSet` →
    in the set; never the requester) — must number ≥ the policy's `approvals`. Otherwise the
    result is `NeedsApproval` (not `Refused`). The **reversal path never evaluates this
    invariant** (a staff reversal is not a new spend — an `ApprovalRule` must not be a shield
    against clawback). Approvals authorize an **intent, not a state**: `expected_*_revisions` are
    outside the hash, so a request approved at revision N still authorizes the plan at N+k.
18. **Scope/owner coherence** (assets, vaults, and — at open — accounts): `scope: Some(s)` ⇒ the
    owner is `PrincipalId::Organization(s.org)`; `TransferTitle` to a non-org principal requires
    `to_scope: None`. Checked in `validate` for asset ops and at `@vault/create`/`@account/open`.
    *As built, split:* the pure engine proves the structural half (a scoped resource is org-owned; a
    non-org destination carries no scope); **which** org (`s.org == owner`, and both Active) needs
    `OrgState`, so the seam (`authorize_asset` / `authorize_title_receipt`, §4.4) proves it.
    There is deliberately **no asset-value escalation** in invariant 17: rules live on accounts and
    fire on net debit, so an effect-only plan moves a title of any value with no approval —
    two-party consent (E4) is the control for that, not a threshold.
19. `Encumber` is **staff-only in E3** (a `Lien` without a `ContractId` cannot represent a real
    obligation; an unpriced lien is a hostage primitive). `Release(Lien)` — its `creditor` or
    staff; `Release(Disputed)` — staff. Both checked via `AuthEval::authorize_asset`.
20. **Both sides of a title transfer need authority** (D-E3-12): `TransferTitle` requires
    `authorize_asset` on the *source* (owner / `asset.title.transfer` on its scope / staff) **and**
    on the *destination* (to an org → `asset.title.transfer` on `to_scope`; to an object principal
    → the actor *is* that principal, or staff). `Title` (create) is staff-only.
21. Boundedness: the entry probe (`entry_probe`, `engine.rs:206-220`) serializes the effects too,
    so an accepted plan provably fits `MAX_ENTRY_BYTES`; `MAX_REASON_BYTES` caps every free-text
    reason inside an effect (the full-post-state form re-serializes them on every later effect).
22. `acquisition` is stamped `{ transaction, at }` on `Title`/`TransferTitle` whenever the same
    transaction carries ≥1 money line (§9.4 provenance seed).

### 3.6 Seams the pure engine takes

```rust
pub trait AuthEval {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool;                       // E2
    fn authorize_asset(&self, _actor: &PrincipalId, _asset: Option<&EconomicAsset>, _op: &AssetOp) -> bool { false }
    fn authorize_title_receipt(&self, _actor: &PrincipalId, _to: &PrincipalId, _to_scope: Option<OrgScopeId>) -> bool { false }
    fn authorize_vault(&self, _actor: &PrincipalId, _vault: &Vault, _op: &CustodyOp) -> bool { false }
    fn authorize_maintenance(&self, _actor: &PrincipalId, _account: &Account) -> bool { false }
    fn authorize_approver(&self, _approver: ObjId, _account: &Account, _policy: &ApprovalPolicy) -> bool { false }
}
pub trait CustodyEval {
    fn backing_valid(&self, vault: &Vault) -> bool;
    fn holds(&self, vault: &Vault, obj: ObjId) -> bool;
    fn may_deposit(&self, obj: ObjId, actor: &PrincipalId) -> bool;
    fn destination_ok(&self, actor: &PrincipalId, to: ObjId) -> bool;
}
pub struct ValidateEnv<'a> { pub auth: &'a dyn AuthEval, pub custody: &'a dyn CustodyEval, pub now: i64 }
pub fn validate(plan, snapshot, env: &ValidateEnv<'_>) -> PlanResult;   // as built: the clock is a
                                                                        // real new dependency (approval expiry)
```

Parameter names are underscored in the default bodies (CI runs clippy with `-D warnings`).
Default `false` keeps every foreign impl fail-closed; `AllowAll` overrides to `true` (note in its
doc that it now authorizes *title*, not just money — tests/trusted only); a `NoCustody` fake
(everything invalid) ships for tests. `plan_reversal` (`engine.rs:283-330`) **keeps its E2
signature**, builds an in-crate `ValidateEnv { auth, custody: &NoCustody, now: original.posted_at }`
(nothing on that path reads the clock) with `skip_approval = true`, and reverses **money lines
only**; the reversal entry has empty effect vectors. **`reverse` refuses a
target whose `asset_effects`/`custody_effects` are non-empty** (`EconomyError::HasEffects`), naming
the compensating `@asset/transfer` — no `/force` in E3.

Blast radius of the `validate` signature (grounding review): `src/api.rs:164`, `tests/e2a.rs:56,
62, 123, 149, 301` — six call sites, all in-crate.

### 3.7 `plan_hash` v2 (`hash.rs`)

`CanonicalPlan` gains `asset_ops` and `custody_ops`, both `#[serde(skip_serializing_if =
"Vec::is_empty")]` so **an E2-shaped plan hashes byte-identically under E3** (journal-derived
idempotency for pre-upgrade entries keeps working; boot never recomputes a stored hash). Canonical
order is a **total order**: asset ops by `(object, op discriminant, canonical bytes of the op)`;
custody ops by `(object, vault, discriminant)` — with one custody op per object that is total.
`expected_asset_revisions` is excluded like `expected_revisions` (R-B3). `reversal_hash` unchanged.
The regression pin compares against a **hard-coded digest** of a fixed money-only plan, so a
later field reorder cannot pass silently.

### 3.8 E3a tests (pure, `tests/e3a.rs`, target ~30)

Every op on hand-built snapshots; every invariant 10–22 refusal; `Release`+`TransferTitle` in one
plan succeeds, two title-changing ops on one object refuse, two plans differing only in op order
hash identically; money+title applies both, the effect carries the post-state, `acquisition` is
stamped only when money moved; transfer clears assignment + maintenance; asset revision bumps once
per effect; `NeedsApproval` fires above threshold not at/below; an approved request whose approver
no longer qualifies (`authorize_approver` now false) no longer satisfies; mutate the plan →
`NeedsApproval` again (scenario 11 pure half); expiry; reversal of an effect-carrying tx refused;
reversal path ignores `ApprovalRule`; hash v2 == hard-coded E2 digest
(`9c917eee84e001912480fdb8306b9a762ed2e3f50d1014e34ae38601c490d74e`, computed on the pre-change
code); fuzz (E2a's deterministic xorshift, not proptest — the crate has no proptest dev-dep): random
op sequences never panic and never yield a transaction with zero lines and zero effects.

---

## 4. E3b1 + E3b2 — the live module

### 4.1 Persistence + boot (`api.rs`, `boot.rs`, `module.rs`)

New docs under `NS = "economy"`: `asset:{obj}`, `vault:{id}`, `approval:{id}`; `meta` gains
`next_vault_id`, `next_approval_id`, and **high-water marks boot never lowers**.

| Doc | Polarity | Boot rule |
|-----|----------|-----------|
| `asset:{obj}` | **projection** (D-E3-1) over journal-authority fields; `frozen` latch is doc-authority | derived from the journal's last `AssetEffect` per object. Drift compares **journal-authority fields only** (never `frozen`/`frozen_reason`). Lagging doc → heal silently. Doc disagreeing with the journal at its claimed `last_journal_seq`, or ahead of it → **freeze economy**. **Missing doc for a journaled asset → rebuild + `frozen = true`** ("doc lost") — never a silent heal of the latch. Doc with no journal history → **not loaded**, loud warning (an unjournaled doc is not a title; the next legit `Title` overwrites it) |
| `vault:{id}` | container | corrupt → fail boot; backing object missing/`GOING`, or `backing_valid == false` → `Frozen` + re-persist (never delete) |
| `approval:{id}` | leaf | corrupt → skip + **staff-visible warning naming the key**; `Pending` whose id appears as some transaction's `consumed_approval` → `Consumed` (heal — keyed on the id, not the hash); past `expires_at` → `Expired`; `Expired`/`Cancelled` older than 7×TTL → deleted (GC) |
| liveness | per asset | titled object, `title_owner`, `assignment.assigned_to`, or any encumbrance `creditor`/`claimant` that is an object principal and is missing/`GOING` → `frozen = true` + re-persist — the boot half of the backstop (only `@destroy` emits `ObjDestroyed`, `mush-core/src/commands/building.rs:688-694`) |
| custody reconciliation | per deposited object | replay `custody_effects` to a last-known-vault map; for each object whose last effect is `Deposit`, ask the provider `holds(vault, obj)`; mismatch → if titled, `frozen = true` (reason `custody-mismatch`); always a loud log. Custody effects are **audit-only** — no boot path, index, or query derives a location from them |
| allocator floors | | `next_vault_id ≥ max(custody_effects.vault)+1`; `next_approval_id ≥ max(consumed_approval)+1`; both also `≥` the meta high-water mark |

`commit_prepared` (`api.rs:222-249`) additionally persists every touched `asset:{obj}` projection
and the `Consumed` approval doc; custody truth stays in the world (`Obj.location`/`SpatialParent`),
which the same `WriteBatch` carries (`mush-testkit/src/lib.rs:236-244`).

*As built, the latch rule made precise:* **a doc-authority latch write (asset `frozen`, vault or
account status, an approval rule) touches no journal-authority field — `revision` included.**
Bumping `revision` on a backstop freeze makes the doc disagree with the journal at its own claimed
seq, and the next boot reads the freeze itself as drift (the C2 regression caught it in test).
Two further boot rules: a *corrupt* `asset:{obj}` doc fails the boot (its latch is unreadable, and
proceeding would silently un-freeze a recovering title); a doc claiming a `last_journal_seq` at
which its object has no effect is "disagrees at its claimed seq" → freeze. A vault whose
`backing_valid` is `None` at boot is left alone with a warning (only a missing/`GOING` backing or a
definite `Some(false)` freezes it). Persistence and authorization for title land **together**
(E3b1): E3a alone left every asset op refused by default, so title could never be live-but-undurable.

### 4.2 Post ordering — the atomicity rules

A handler that returns `Err` **discards its deferred persists but not its world mutations**
(`mush-core/src/module.rs:1823-1827`). Inside `post`:

1. `validate` (pure; nothing mutates).
2. **Only on `PlanResult::Post`**, and driven **only by `Prepared.custody_effects`** (never by the
   plan's `custody_ops` — an `AlreadyPosted` retry performs zero world mutation), run each custody
   move through the resolved `CustodyProvider`. The provider **must** fully validate before
   mutating (`can_move_to` + carry hooks then `move_to` for containers; `mush_sim::plan` then
   `apply` for interiors). A provider `Err` returns before the store is touched.
3. `apply` to the store snapshot + `index_transaction` (infallible).
4. Queue the bundle: journal append + account docs + asset docs + approval doc + meta.
5. `gate_on_durable_ack_strict()`.
6. **`post` is the last fallible call in the handler.** Commands parse and resolve everything
   before step 1; nothing after `post` returns may `?`. This is `submit`'s documented contract
   (`mush-org/src/api.rs:169-171`) and goes into `post`'s doc comment.

Step 3 must never precede step 2 (a custody failure after a RAM store advance leaves RAM ahead of
the discarded persists). Regressions: a provider `Err` after a valid money plan mutates nothing;
a handler that `Err`s after a successful `post` leaves store, world, and persists consistent.

*As built:* the engine admits up to 32 custody ops, but with the single-call provider contract a
provider that refuses the *k*-th move would leave the first *k*−1 applied. **`post` therefore
refuses more than one custody effect per plan at the module layer** (E3b2), until a two-phase
provider API exists. The deposit-side `claims` check (is some *other* provider the object's
owner?) also runs here, at step 2 before any mutation — `CustodyEval::may_deposit` has no vault
argument and so cannot resolve the backing; the withdraw side resolves through `holds` at validate.

### 4.3 The custody seam (`custody.rs`, new)

```rust
pub trait CustodyProvider: Send + Sync + std::fmt::Debug {
    fn provider(&self) -> &str;                                              // "sim"
    fn handles(&self, backing: &CustodyRef) -> bool;
    /// `true` if this provider owns the object's placement/motion (sim: placed, quarantined,
    /// host/space/mount/station-bound — `is_sim_bound` — OR mid-journey, i.e. sim's own
    /// `gate_reason` predicate, not half of it).
    fn claims(&self, w: &World, obj: ObjId) -> bool;
    fn backing_valid(&self, w: &World, backing: &CustodyRef) -> Option<bool>;
    /// Transitive, depth-capped at MAX_CONTAINMENT_DEPTH (a container deposited whole is not a
    /// laundering primitive: its contents are visible and individually withdrawable).
    fn contents(&self, w: &World, backing: &CustodyRef) -> Option<Vec<ObjId>>;
    fn holds(&self, w: &World, backing: &CustodyRef, obj: ObjId) -> Option<bool>;
    /// Validate-then-mutate; MUST NOT half-apply (§4.2).
    fn move_into(&self, ctx: &mut Ctx<'_>, actor: ObjId, obj: ObjId, backing: &CustodyRef) -> Result<(), CustodyError>;
    fn move_out(&self, ctx: &mut Ctx<'_>, actor: ObjId, obj: ObjId, backing: &CustodyRef, to: ObjId) -> Result<(), CustodyError>;
}
```

`EconomyModule::with_custody(Vec<Arc<dyn CustodyProvider>>)` freezes the table (the
`OrgModule::with_predicates` pattern, `mush-org/src/module.rs:172`); the module always appends the
built-in `ContainerProvider` **last**. **Resolution: exactly one provider resolves a whole op — the
first whose `handles(backing)` is true; the op is refused if any *other* registered provider
`claims(obj)`.** `None` from any query = fail closed.

`ContainerProvider` (handles `CustodyRef::Container`, claims nothing): `contents` via
`World::contents_of` (`world.rs:469`) transitively; moves via `can_move_to` (`:635`) **and the
`CarryAttempt` hook chain** (`mush_core::module::check_carry_attempt`, `module.rs:1632-1641` — the
only thing that makes sim's `PlacementGate` see the move; `World::move_to` itself consults no
hook) **and** a `MAX_CONTAINMENT_DEPTH` (`world.rs:35`) check on the resulting chain (`move_to`
does not enforce it and `containing_room` returns `None` past it), then `move_to` (`:935`).
`Registered{..}` backings have no provider in E3 and are always invalid. `@vault/create` refuses
`interior:` until a registered provider `handles` it (E3c), and refuses a backing that any
provider **other than the one resolving it** `claims` (an interior vault's backing *is* a space
sim claims, so the resolving provider is exempt — *as built*).

`WorldCustody { w, providers }` bridges the table to E3a's `CustodyEval`, as `WorldAuth` bridges
`AuthEval` (`api.rs:82-118`).

### 4.4 Authorization (`api.rs` `WorldAuth`)

New exact-match capabilities (E1 matches `CapabilityId` by equality only, `authorize.rs:149`, and
`:167` for role templates): `economy.account.view`, `economy.account.approve`,
`asset.title.transfer`, `asset.assign`, `vault.view`, `vault.deposit`, `vault.withdraw`. (No
`asset.encumber` — invariant 19.) `mush_economy::CAPABILITIES` exports the vocabulary for §4.8.

- **Money.** The E2 rules, plus: **a negative delta on a `System`-owned account requires staff**
  (moved *into* the bridge from the command layer — the old `api.rs:101-103` "gated by the command
  layer" clause fails open the moment `econ_pay`'s owner guard is relaxed for divisional wallets).
  `econ_pay`'s guard becomes "owns it, **or** the account is scoped" — never "let the engine
  decide". A scoped debit additionally requires the org `Active` and the scope `Active` (checked
  in `WorldAuth` *and*, after E3b2, in org's `authorize`). Credits: to a `Dissolving` org allowed
  (final settlement, §5.2); to `Suspended`/`Closed` refused.
- **Asset ops:** staff (`SIM_ADMIN`/`Royalty`, `api.rs:95-99`) → allow. Otherwise the actor is the
  `title_owner` (object principal) or, for an org-owned asset, holds the capability on `asset.scope`
  via `authorize_in_world`: `asset.title.transfer` for `TransferTitle`/`WriteOff`, `asset.assign`
  for `Assign`/`Unassign`, `asset.title.transfer` for `SetMaintenance`. Receipt per invariant 20.
- **Custody ops:** vault owner (object principal), or `vault.deposit`/`vault.withdraw` on the
  vault's scope, or staff — plus custody standing (invariant 15). Vault ops never consult title
  (D5; theft is spatial).
- **Approvals:** `authorize_approver` as invariant 17; the requester never counts. `Threshold {
  approvals: 1 }` therefore means *one other person*. `@account/approval` refuses `from_role` for a
  role that does not exist in the account's org (role-based approval needs real `OrgMembership`,
  D-E3-11) and refuses `OneAuthorizedActor`.
- **Limits.** `GrantLimit` (`authorize.rs:284-291`) is the **org-side per-operation ceiling**;
  `ApprovalRule` is the **account-side per-operation escalation**. Neither is a budget: ten
  payments of 1 000 pass a 10 000 threshold or limit — intended until holds (E4), stated in both
  commands' help and pinned by a test. `@org/grant` refuses `limit=` on a non-amount capability
  (`AuthRequest { amount: None }` + a limit is `AmountRequired` forever).

### 4.5 Reads and existence-hiding (bounded E4 pre-step, D-E3-6)

Non-staff may read what they own or hold `*.view` over: `@account/show|list` (id, instrument,
name, status, `posted_balance`, the approval rule if any), `@asset/show|list` (object, title
owner, lifecycle, assignment, **count** of encumbrances), `@vault/show|list` (id, name, owner,
status, provider-derived contents to `vault.view` holders — a live query, never a stored list;
contents ≠ title). Everything else is staff or E4's filter. The journal stays staff-only.

**The existence-hiding rule lives in the command layer's error renderer, not per verb:** any
refusal (`EconomyError` or gate) touching an account/asset/vault the actor is not authorized to
*view* renders as that resource's "No such …" message; only a view-authorized actor sees the
specific error. Every `*/list` with an unauthorized, hidden, or nonexistent owner/scope filter
returns an empty list. Lists cap at 20 rows + "N more". `@econ/approve|deny` on an id the actor
may not approve renders as an unknown id; status-specific messages come only after standing is
established. A `NamedSet` approver is granted view of the request's accounts for its lifetime.
The destroy gate's refusal is **neutral for non-staff** ("You cannot destroy that right now.") and
detailed for staff.

### 4.6 Org-scoped wallets + approvals — the divisional-wallet vertical (E3b2)

- `@account/open org:<org>[/<scope>]=<instrument>` (staff) → `Account { owner: Organization(org),
  scope: Some(scope) }`; scope must belong to the org and be `Active` (invariant 18); default =
  `root_scope`. `@account/approval <id>=<threshold> <policy>` with `policy` = `threshold:<n>[:<role>]`
  | `named:<n>:<obj>,<obj>…` | `none`. `@account/close <id>` (zero balance, `Active`; org accounts:
  staff or post authority). `@account/unfreeze <id>` (staff recovery; E2 shipped no unfreeze).
- `post` on `NeedsApproval`: if a live request for this hash exists, report it; else mint one
  (`Pending`, `expires_at = now + APPROVAL_TTL_SECS` = 86 400), persist, non-strict gate (no money
  moved). Caps: `MAX_PENDING_PER_REQUESTER` (4), `MAX_PENDING_PER_ACCOUNT` (16) — refuse beyond.
  Re-submitting the same plan after enough live-qualifying approvals posts it and marks the
  request `Consumed { transaction }` in the same bundle; `consumed_approval` rides the journal.
- `@econ/approve <id>` / `@econ/deny <id>` (standing as §4.4; each records a `Decision`; deny →
  `Cancelled`); `@econ/approvals` lists requests the actor may act on, indexed by account scope,
  capped. *As built:* **standing == counting** — `may_act_on_request` *is* `authorize_approver`, so
  an actor who could never count (capability but not the role) sees the unknown-id text rather
  than recording a dead decision. `NamedSet` membership is necessary **and sufficient** (requiring
  the capability as well would make a named set inert without extra grants and the §4.5 view
  grant pointless); staff qualify for `Threshold` but never for `NamedSet` unless named. The GC
  anchor is `expires_at + 7×TTL`; `Consumed` requests are never collected. Credits into a
  `Suspended`/`Closed` org's wallet have **no staff exemption** — staff reactivate the org first.
  The `MAX_PENDING_PER_ACCOUNT` cap is only reachable across several requesters (the per-requester
  cap fires first) and is tested that way.
- Scenario 11 end-to-end: rule `threshold:2`, pay above threshold → request; one approval → still
  needs one; edit the amount → the old request does not authorize the new plan; revoke an
  approver's capability → the request no longer satisfies; two identical submissions never mint
  two requests.

### 4.7 Commands (`commands.rs`)

Every money/title/custody verb goes through `post`; there is no direct-mutation path.

```text
@asset/title    <obj>=<principal>[ scope=<id>]         staff: create a title record
@asset/transfer <obj>=<principal>[ scope=<id>]         invariant 20 (both sides)
@asset/assign   <obj>=<principal>[ purpose[ until]]     @asset/unassign <obj>
@asset/maintenance <obj>=<account|none>                invariant 14
@asset/lien     <obj>=<creditor> <amount> <instr>      staff (invariant 19);  @asset/release <obj>=<n>
@asset/writeoff <obj>=<reason>    @asset/unfreeze <obj>  staff (unfreeze is a doc write, not journaled)
@asset/show <obj>   @asset/list [owner=|scope=]        §4.5
@vault/create <name>=<container:<obj>|interior:<obj>>[ owner=<principal>][ scope=<id>]
@vault/deposit <obj>=<vault>   @vault/withdraw <obj>=<vault> [to=here]
@vault/show <vault>   @vault/list   @vault/freeze|unfreeze <vault>   @vault/close <vault> (empty only)
@econ/pay <from>=<to> <amount> [asset=<obj>] [from_vault=<id>|to_vault=<id>]   one plan; arity 1
@econ/approve|deny <id>   @econ/approvals
@org/scope create <org>=<name>[ parent=<scope>]   @org/scope retire <org>=<scope>
@org/grant <obj>=<org> <cap> [scope=<id>] [limit=<n>]
```

`<principal>` parses `#obj`, `org:<id|name>`, `sys:<name>`; keys via the E2b `cmd_key` pattern
(`commands.rs:94-97`). The two-party refusal for `@econ/pay … asset=` names "counterparty consent
is E4" rather than a bare denial. `@chown` is **not** gated and does not touch title (§9.1
independence, tested both ways).

### 4.8 `mush-org` changes (E3b2, separate commit)

1. `authorize` denies (`DenyReason::Inactive`) when the target scope's org is not `Active` or the
   scope is `Retired` — right after the `NoSuchScope` short-circuit (`authorize.rs:135-137`).
   Both statuses are inert today; E1's 34 tests may assume it — note in the commit.
2. `OrgMutationOp::RetireScope { scope }` — arms in **both** `plan.rs` `apply_one` (`:176-311`)
   and the `api.rs` persist dispatcher (`:203-219`); a root scope cannot be retired. Refused while
   any account/asset/vault binds the scope — *as built* through a new predicate-table-shaped seam,
   `mush_org::ScopeUseProvider` (frozen at wiring via `OrgModule::with_scope_use`), implemented by
   `mush_economy::EconomyScopeUse`; org cannot see the economy, so it asks. It fails open only if
   no economy module is registered at all.
3. `MAX_SCOPES_PER_ORG` (64) in `CreateScope` validation.
4. **Capability vocabulary** (08a decision 6 — E3 is the consumer): `OrgModule::with_capabilities
   (BTreeSet<CapabilityId>)` frozen at wiring (empty = accept all, the E1 behavior); `@org/grant`
   refuses an unknown capability when a vocabulary is configured; `@org/show` flags unknown ones.
   The server wires `mush_economy::CAPABILITIES ∪ {org.role.manage, org.grant.delegate}`.
5. `@org/scope create|retire`; `@org/grant` gains `scope=` (exact scope selector) and `limit=`.
   `submit` stays authorization-free and non-gated; commands gate.
6. *As built, a follow-up for a later cut:* `Organization.revision` is inert (no op bumps it), so
   `@org/scope create` stamps `created_rev = revision + 1` to stop a `SubtreeFrozen` grant silently
   widening to every runtime-created scope. The proper fix — a revision bump on scope creation plus
   `SetOrgStatus`/`PutRole` ops (the org status tests currently patch the persisted doc and reboot,
   and scenario 9's "rename" is expressed as a second identically named role) — needs its own cut.

### 4.9 Destruction: gate + backstop (E3b1)

- **Gate** — a `mush_core::behavior::DestroyGate` (`behavior.rs:647-652`, registered via
  `reg.destroy_gate`, `module.rs:1339`; consulted by `Registry::check_destroy`, `cmd.rs:678-685`,
  from `building.rs:685` — `@destroy/override` bypasses only `SAFE`, never this). Refuse when the
  target, **or anything it transitively contains** (depth-capped): (a) has an `EconomicAsset` not
  `WrittenOff`; (b) backs an `Active` vault, or a `Frozen` vault whose backing still holds
  anything; (c) owns any account with a non-zero `posted_balance` or holds title to any
  non-`WrittenOff` asset (§15.3's fifth gate; the E2 freeze stays the backstop). Message per §4.5.
- **Backstop** (`ObjDestroyed` observer extending E2b's `on_obj_destroyed`, `module.rs:160-187`):
  `frozen = true` on every asset **on** the dead object, **owned by** it, **assigned to** it, or
  naming it as creditor/claimant; `Frozen` on every vault backed by it; the E2 account freeze
  stays. Doc writes, not journal effects (a reduction of authority). Never delete, never transfer,
  never cancel a lien. The observer runs *after* `World::destroy` has already dispersed contents
  to `home`/`LIMBO` (`world.rs:880-895`) — it cannot see what a vault lost; the boot custody
  reconciliation (§4.1) is what catches that.
- **Boot half:** the liveness + custody-reconciliation passes in §4.1.

### 4.10 Server wiring

`mush-server/src/modules.rs:84`: `EconomyModule::with_custody(vec![])` in E3b; E3c adds
`Arc::new(mush_sim::InteriorCustodyProvider)`. Org (`:79`) gains `.with_capabilities(..)` and
`.with_scope_use(..)` in E3b2. **Module order matters for boot (*as built*, E3c): `EconomyModule`
now boots after `SimulationModule`.** `SpatialState` is installed at *registration*, so an economy
booting first sees an *empty* sim state and gets a definite `Some(false)` from the interior
provider — every interior-deposited title would freeze as a custody mismatch and every interior
vault as invalid, on every reboot. Org still precedes economy; scenes/sim read no economy state at
boot. A cold-reboot test pins the old failure mode.

### 4.11 E3b tests (`tests/e3b.rs`, `TestWorld`-driven like `e2b.rs`; ~20 per sub-cut)

**E3b1 (18–22):** **18** transfer title, object does not move; `@teleport` it, title unchanged;
`@chown`, title unchanged; `@asset/transfer`, `Obj.owner` unchanged. **19** assign an org vehicle
to a pilot, revoke; title + location unchanged. **20** container vault deposit + withdraw with
`drain_write_batch().asserts_atomic_with_ledger(obj, "economy", "journal")`; a provider `Err` and a
*policy* refusal each mutate nothing; a carry-hook refusal (stub hook) mutates nothing; a placed
object routed to the container provider is refused; depth cap; withdraw to a vault backing
refused; deposit of a container exposes its contents. **21** `@destroy` refused while titled and
while containing a titled object; `World::destroy` + event → frozen; `World::destroy` without event
+ reboot → frozen; vault backing destroyed → vault Frozen, titles intact; freeze via event,
`fold_kv`, reboot → economy **not** frozen and the asset still frozen (the C2 regression);
teleport out of a vault + reboot → custody-mismatch freeze. **22** after each step of
title→assign→deposit→withdraw→transfer: `fold_kv` + `reboot`, snapshot identical; the asset
projection lag / drift / ahead / missing-doc quartet (**extends** the E2 pair at `e2b.rs:130,152`;
back-fill the E2 "ahead" and "missing account doc" cases too); reversal of an effect-carrying tx
refused; frozen economy refuses asset ops.

**E3b2 (8–12):** **8** two divisional accounts + vaults; the quartermaster with
`economy.account.post` on `logistics` pays from it, cannot pay from `operations`, and every
`show`/`deposit`/`withdraw`/`approve` against the sibling renders the nonexistent-id text;
non-staff debit of a system account refused. **9** rename a role → nothing changes; replace its
grant → next post refused. **10** delegate narrowed, expire the source → refused; the same with
the delegate as *approver*. **11** as §4.6. **12** hidden org: lists, errors, the destroy-gate
message, and list filters do not name it. Plus: `Retired` scope / inactive org refuse debits, a
`Dissolving` org accepts credits; splitting a payment under a threshold is intended;
`@account/close` then `RetireScope`; unknown capability refused at `@org/grant`.

---

## 5. E3c — the sim and tactics seams

### 5.1 `mush-sim::InteriorCustodyProvider` (new file `custody.rs`, commit 1)

Implements `mush_economy::CustodyProvider` for `CustodyRef::SpatialInterior(space)`:
`handles` = `SpatialInterior`; `claims(w, obj)` = sim's own gate predicate (`is_sim_bound(obj)`,
`data.rs:517`, **or** traveling — the `gate_reason` shape at `lib.rs:161-165`); `backing_valid` =
the `InteriorSpaceRecord` exists, its object live, not quarantined; `contents` = `children_of`
(`data.rs:538`) filtered to `SpatialParent::Interior`, transitive through nested spaces;
`holds` = `parent_of(obj)` is `Interior { space, .. }`.

`move_into`: the object must already carry `sim.participant` (`Transition::Place` requires it,
`transition.rs:253-257`; the provider attaches no behaviors — refuse with a clear message
otherwise), then `mush_sim::run(ctx, &Transition::Place { obj, parent: Interior { space, slot: None } })`
(`transition.rs:520`). `move_out`: **pre-validate `to`** (`live` + `w.can_move_to(obj, to)` +
**every `CarryAttempt` hook except the simulation's own** — sim's `PlacementGate` vetoes any move
of a *placed* object, which is the precondition of every withdrawal, so running it would make
withdrawal unsatisfiable, and running it after the `Unplace` would let a foreign gate refuse too
late to be atomic; *as built*), then `run(ctx, &Transition::Unplace { obj, reason, by: Some(actor),
keep: true })` — `keep: true` pushes **no** projection, so `Obj.location` stays on the space object
and the record becomes non-quarantined `Unplaced` (`is_sim_bound` false) — then the single
`move_to(obj, to)`. The `keep: false` form validates only the recovery-room hop and appends the
recovery-log line before the second move, so a refusal there strands the object with an audit
entry claiming a withdrawal that never happened. `backing_valid` is `None` only when the sim
module is **not registered**, never "state not loaded" (the §4.10 order fix covers that).
`mush-sim/Cargo.toml` gains `mush-economy`. The sim `RECOVERY_LOG` audit (`transition.rs:512-515`)
co-commits with the journal. `@vault/create … interior:` is enabled in this commit; the server
wires the provider. Known and inert: a withdrawal leaves a non-quarantined `Unplaced`
`placement:{obj}` doc behind, exactly as `@spatial/unplace` does; no cleanup pass was added.

### 5.2 `mush-tactics` asset scrub (commit 2)

- `UnitMutationOp::SetComponentAsset { component: ComponentId, asset: Option<ObjId> }` — arms in
  `apply_op` (`unit.rs:651-698`, exhaustive) and a new `JournalEntry` variant; mutation via the
  existing `self.components.get_mut(component)` pattern (`:677`).
- `on_obj_destroyed` (`module.rs:150-157`) additionally scans every unit for components whose
  `asset == obj`, nulls them through a `UnitMutationPlan` (revision bump — a queued order that
  captured `(subject, gen)` sees it), and persists the touched units.
- `boot_units` (`module.rs:175-234`) does the same for pointers to missing/`GOING` objects.
- No `mush-economy` dependency. Tests in `mush-tactics/tests/module.rs`: scrub via event and via
  reboot.

### 5.3 E3c tests

Interior vault: deposit a participant-bound unit into a hangar interior + withdraw, atomic with
the journal; a non-participant deposit refused; a sim-placed object offered to a container vault
refused; quarantined space → vault invalid (*as built:* no live command path can quarantine a
space today — a space is a Room and holds no placement record — so the test plants the record
directly); destroy the space → vault Frozen, placed contents keep title; the cold-reboot
module-order regression (§4.10). **Operation axis (D5):** `@asset/assign`/`@asset/transfer` on a crewed unit leave
`StationAssignment` seats and `control_generation(unit)` unchanged; `@spatial/assign` a pilot
leaves `EconomicAsset.assignment`/`title_owner` unchanged.

---

## 6. E3d — faction linkage and the resources migration

### 6.1 The mapping (D-E3-8)

`Faction` (`mush-faction/src/data.rs:236-286`) gains `#[serde(default)] pub org: Option<OrgId>`
and `#[serde(default)] pub migrated: BTreeSet<SmolStr>`; the single struct literal is
`Faction::new` (`data.rs:292`). One org backs at most one faction — checked at bind; at use, a
duplicate binding found live makes **both** links unresolvable (fail closed, `@faction/show`
prints `org #N (CONFLICT)`) until staff re-bind. The link is validated at every use (a missing org
→ the use fails closed; `org #N (MISSING)`), not at faction boot — `FactionModule` loads before
`OrgModule` (`modules.rs:73` vs `:79`) and E3 does not reorder.

- `@faction/org <faction>=<org|new|none>` (staff): bind, create-and-bind (`new` → the `@org/create`
  path via `submit`; *as built* the **org** carries the faction's name and its root scope is named
  `root` exactly as `@org/create` does), or unbind. Unbinding leaves the org and its accounts
  standing. `@faction/create` does **not** auto-bind — money attribution is a deliberate staff act.
  *As built:* one resolver, `mush_faction::backing_org(w, &Faction) -> Result<OrgId, LinkError>`
  (`Unbound` / `Missing` / `Conflict`), is used by every consumer; `@faction/destroy` reads the raw
  link, so a `MISSING` or `CONFLICT` link also blocks destruction.
- `@faction/destroy` (`commands.rs:216-300`) is **refused** while `org` is `Some`.

### 6.2 The resource rule (D-E3-9, static)

Classification is **explicit, one-way, per key**: a key is money iff it is in `Faction.migrated`.
Registering an instrument never reclassifies anything.

- `@faction/migrate <faction>=<key> <sysacct> [/confirm]` (staff, per key): the key must equal an
  **`Active` instrument's code**; the faction must have a resolvable backing org; the rating
  integer is taken **verbatim as minor units** and the command prints `to_decimal_string` and
  refuses without `/confirm`. On confirm: open the org's root-scope account for that instrument if
  missing, **`post` first** (idempotency key `faction.migrate/{fid}/{code}`, cause
  `Staff { reason: "faction resource migration" }`, from the named system source), then remove
  the key, insert it into `migrated`, and append a `ResourceChange { delta: -old, reason:
  "migrated to account #N" }` **directly to `resource_log`** (not via `apply_resource`, which would
  resurrect the key at 0), persist the faction doc. Post-then-mark converges: a dropped faction-doc
  write reboots with the key present, and a re-run is an idempotency HIT that just re-marks.
  *As built (after the §13 fold):* the source account must hold the key's instrument and be
  `Active`, the backing org must be `Active`/`Dissolving`, and the economy unfrozen — all checked
  **before** the org wallet is opened, so nothing fallible follows that store mutation (and if
  `post` still refuses, the refusal is reported and the opened wallet's persist kept). The
  idempotency record `faction.migrate/{fid}/{code}` is consulted **before** any pool judgement: if
  it exists the migration already posted, so the command marks the key, removes it, logs the
  residual (`-current_pool`, only when non-zero), and reports the original transaction — a pool
  edited or spent between the post and the re-run still converges.
- `@faction/resource <f>=<kind>/<delta>` refuses a `migrated` key with a pointer to `@econ`;
  `Faction::apply_resource` (`data.rs:310-326`) keeps its saturating arithmetic and is now
  provably rating-only.
- `@faction/show` and `+faction` (`commands.rs:1015-1017`, `:1375-1379`) render the backing org's
  root-scope balances **only** to staff or holders of `economy.account.view` on the root scope
  (nothing — not "hidden" — otherwise); the faction officer gate governs ratings only.
- The sim never reads resources (`sim.rs`, zero hits).

### 6.3 E3d tests

Bind/unbind/new; conflict → both unusable; destroy refused while bound; a rating key still
applies; a migrated key is refused; migrate refuses without confirm, refuses an inactive
instrument, posts exactly once (second run is a HIT that changes nothing); post, drop the faction
doc, reboot, re-run → balances unchanged, key gone; `tests/faction.rs:548-585` splits into a rating
case and an account case; a faction officer with **no** `OrgMembership` and no grant sees ratings
and no money, and cannot post from the org account (D-E3-11 documented deny); reboot after migrate
reproduces balances from the journal.

---

## 7. Build discipline per cut

1. Re-read this plan's slice for the cut; confirm its D-E3 items still hold against the tree.
2. Build library first, module second, tests alongside; `cargo fmt --all`, `cargo clippy
   --workspace --all-targets -- -D warnings`, `cargo test --workspace` (the CI gates,
   `.github/workflows/ci.yml`).
3. Commit per cut (E3b2's org change and E3c's two halves as their own commits), E1b/E2b message
   shape, nothing pushed.
4. After E3d: the adversarial verify over the whole E3 diff — three agents hunting (a) fail-open
   authorization, (b) title/custody/money desync, (c) durability/boot (freeze polarity, allocator
   floors, an E2 journal under the E3 binary) — then fold every confirmed hole with a regression
   test, update 08a's ledger and decisions, commit.

## 8. Decisions (review-confirmed unless marked)

- **D-E3-1 Journal-carried asset effects; `asset:{obj}` is a projection; `frozen` is a doc-authority
  latch outside the effect.** A companion-doc drop can never desync title from the money that
  bought it; scenario 22 follows from replay. The latch split is the Account discipline (E2 boot
  compares `posted_balance`, never `status`); a lost doc rebuilds *frozen*. *Alternative rejected:*
  plain docs in the same `WriteBatch` (§7.2's wording) — a dropped doc write silently loses a
  paid-for title.
- **D-E3-2 Approval rules on the `Account`, requests in `mush-economy`, approvers re-qualified live,
  one live request per hash, four-eyes.** The threshold is an `Amount` in one instrument (a grant
  may span instruments); the request binds `plan_hash`, which E2a owns. `Satisfied` is derived,
  never stored. Doc 08 §15.1 placed `approval:{id}` under org — amend it.
- **D-E3-3 `CustodyProvider` seam, E1c-shaped, sim implements it; single-call validate-then-mutate
  with the §4.2 ordering rule.** One provider resolves a whole op; moves ride the carry-hook chain.
- **D-E3-4 Single-authority plans in E3.** Counterparty consent is E4.
- **D-E3-5 No `PolicyRef`; per-resource policy = a leaf scope + `AccessPolicy` rules against
  `ScopeSelector::Exact`; assets gain `scope`; invariant 18 ties scope to owner.**
- **D-E3-6 A bounded non-staff read with a renderer-level existence-hiding rule.** Forced by
  scenario 8's "undiscoverable"; the journal stays staff-only.
- **D-E3-7 The destroy gate covers principals, assets, vaults, and transitive contents** (§15.3's
  gates verbatim; the E2 freeze remains the programmatic backstop). Safe to be strict because
  D-E3-12 removes the unsolicited-title pinning vector.
- **D-E3-8 `Faction.org: Option<OrgId>` on the faction doc, validated at use; conflict = both
  unusable; destroy refused while bound.**
- **D-E3-9 Static, per-key migration marking (`Faction.migrated`) with an explicit confirmed staff
  command; verbatim minor units.** A registry-equality classifier would let `@instrument/register`
  retroactively turn a rating into mintable money.
- **D-E3-10 `plan_hash` v2 via skip-if-empty, total-ordered ops, hard-coded regression digest.**
- **D-E3-11 The E1c membership adapter is struck.** Role templates need real `OrgMembership`;
  faction-backed orgs authorize via direct grants + `FactionStandingProvider` predicates;
  `from_role` approval policies are refused where the role does not exist.
- **D-E3-12 Title transfer needs authority on both sides; `Title` and `Encumber` are staff-only
  in E3; transfer clears assignment + maintenance.** Title carries liabilities (maintenance,
  un-destroyability) that the money credit rule does not, so "receiving needs no authority" does
  not extend to it.
- **D-E3-13 Title issuance for rulesets** (E5 chargen, E7 production) will be a registered
  title-issuing system principal mirroring §14.1's faucets — one mechanism, built when first
  needed. *(shape only; not built in E3)*

## 9. Constants

`MAX_ASSET_OPS_PER_PLAN` 32 · `MAX_CUSTODY_OPS_PER_PLAN` 32 · `MAX_ENCUMBRANCES_PER_ASSET` 16 ·
`MAX_REASON_BYTES` 512 · `MAX_APPROVERS_PER_REQUEST` 8 · `MAX_PENDING_PER_REQUESTER` 4 ·
`MAX_PENDING_PER_ACCOUNT` 16 · `APPROVAL_TTL_SECS` 86 400 · approval GC at 7×TTL ·
`MAX_SCOPES_PER_ORG` 64 · list cap 20 · `MAX_CONTAINMENT_DEPTH` (core, 8) reused for custody walks.

## 10. Open questions for the owner (non-blocking; defaults chosen)

1. `APPROVAL_TTL_SECS` = 86 400 real seconds — confirm.
2. D-E3-12's "transfer to another character is staff-only in E3" — confirm (the alternative is
   recipient acknowledgement, which is E4's consent).
3. Whether to reorder `FactionModule` after `OrgModule` (would allow link validation at boot).

## 11. Review outcome (Draft 0.1 → 0.2)

Three passes ran on 2026-09-02 against Draft 0.1. Folded, by source:

- **Grounding:** `plan_reversal` keeps its signature (in-crate `NoCustody`); `PostOutcome::NeedsApproval`
  + the three non-exhaustive matches; underscored default params; interior `move_out` half-apply
  (→ pre-validate + `keep: true`); `Place` needs `sim.participant`; `World::move_to` is ungated
  (→ carry hooks in the container provider, `claims` demoted to routing); destroy-gate message is
  visible to non-staff (→ neutral text); `RetireScope` has two match sites; five stale line
  citations corrected; the boot quartet *extends* E2's pair; container depth cap; `carried_by`
  exact-match invariant; no economy dev-dep on tactics.
- **Adversarial:** duplicate requests per hash (A1); unconstrained `Withdraw.to` + hook bypass +
  `GOING` destination (A2); invariant 11/12 contradiction (A3, also design-fit C.7); attacker-chosen
  `to_scope` (A4 → invariant 18); approvers frozen at approve time (A5 → live re-qualification);
  system-account debit fail-open once the pay guard relaxes (A6 → gate in the bridge); approval as
  a clawback shield (A7 → reversal skips invariant 17); title receipt carries liabilities (A8/A9 →
  D-E3-12 + transfer clears relationships); unpriced liens (A10 → staff-only); inert
  `OneAuthorizedActor` (A11 → refused); per-op limits are not budgets (A12 → stated + tested);
  limits on non-amount caps (A13 → refused); `AlreadyPosted` must not drive custody (B1); `post`
  terminal-fallible rule (B2); the `move_out` half-apply (B3); query/mutation provider split (B4 →
  one provider per op); route travelers not claimed (B5); container laundering (B6 → transitive
  contents + gate); custody sort not total (B7); `frozen` un-healable / drift-tripping (C1/C2 → the
  latch split); approval heal by hash (C3 → by id); allocator floors (C4); money-only reversal of
  title (C5 → refused); unjournaled doc bricking the economy (C6 → not loaded); hard-coded hash pin
  (C7); backstop runs after dispersal (E1 → custody reconciliation); gate ignores contents (E2);
  dynamic classifier (F1 → static marking); scale ambiguity (F2 → verbatim + confirm); resurrecting
  the key (F3); inactive instrument (F4); migrate ordering (F5); duplicate binding (F6);
  unbounded approvals/listings/reasons/revision vectors (G1–G5).
- **Design-fit:** membership adapter struck explicitly (A.3 → D-E3-11); `@asset/maintenance` +
  cross-account authority (A.4 → invariant 14); operation-axis tests + no-eviction statement (A.5);
  scenario 12/20 residues (A.6); `acquisition` stamped (A.7 → invariant 22); `@chown` independence
  tested (B.1); non-staff field sets (B.2); `CatalogRef` note (B.3); loud approval-doc drop (B.4);
  capability vocabulary (B.5 → §4.8); effect-only entries and conservation (C.1a); reversal refusal
  (C.1b); `GrantLimit` vs `ApprovalRule` framing, caps + GC, decisions on the doc, reservation
  statement, intent-not-state (C.2); scope/owner coherence (C.3a), coterie-as-org (C.3b), scope cap
  (C.3c); D-E3-7 hedge removed (C.4); static classification + conflict handling (C.5); title
  issuance shape (C.6 → D-E3-13); provider resolution restated (D.3); custody effects audit-only
  (D.4a); the new `mush-sim` edge stated honestly, no feature flag (D.4b); renderer-level
  existence hiding, neutral destroy text, view-gated faction balances, approval-surface privacy
  (E.1–E.4); custody standing replaces "carried" (F.1); withdraw destination (F.2); pay
  composition direction/arity/consent text (F.3); `@vault/show` contents, `@account/close` (F.4);
  transfer clears assignee-equals-owner, reason caps, credit-vs-debit per org status (F.5); E3b
  split (G.1); hook fix removes the hidden E3c prerequisite (G.2); `interior:` refused until E3c
  (G.3); tactics scrub as its own commit (G.4).
- **Not folded (deliberate):** a `mush-sim` `economy` cargo feature; a two-phase custody plan
  across the trait (the terminal-fallible rule plus the module-layer one-custody-op cap make
  single-call safe); reordering the module list — *superseded:* E3c did reorder economy after sim
  (§4.10), for a reason the review did not anticipate.

## 12. Build outcome

Built 2026-09-02/03 on `main` after `99f5b9c` (this plan), each cut by an Opus build agent from the
section named, gated on `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace`, committed locally and not pushed. `cargo fmt` was deliberately not
applied: the tree had 1334 rustfmt hunks at `d7f6707` before E3 touched it.

| Cut | Commit | Tests added | Crate tallies after |
|-----|--------|-------------|---------------------|
| E3a | `e780fd7` | 46 (`tests/e3a.rs`) + 2 hash pins | mush-economy 75 |
| E3b1 | `288f769` | 31 (`tests/e3b.rs`) + 2 E2b back-fills | mush-economy 108 |
| E3b2 (org) | `cb7c8f3` | 12 (e1a +4, e1b +8) | mush-org 46 |
| E3b2 | `ae3de96` | 33 (`tests/e3b2.rs`) | mush-economy 141, mush-server 62 |
| E3c (sim) | `d48b1eb` | 10 (`mush-sim/tests/custody.rs`) | mush-sim 72 |
| E3c (tactics) | `fe4faea` | 4 | mush-tactics 52 |
| E3d | `d8d17c3` | 15 (`mush-faction/tests/economy.rs`) | mush-faction 52 |

Workspace after E3d: 1695 tests pass; the single failure, `mushd::web_e2e::spa_and_healthz_are_served`,
predates E3 (it serves the embedded Vite build and `web/dist` is empty on a fresh clone). Total
E3 diff: 42 files, +12 609 / −303. The end-of-build adversarial verify and its fold are recorded
in §13.

## 13. End-of-build verify outcome

Three Opus verifiers ran over `d7f6707..d8d17c3` on 2026-09-03 with the §7 lenses, each required
to **confirm** a finding with a throwaway probe test (deleted afterwards) rather than by reading.
Folded in the "E3 verify fold" commit `fa97479`, each with a regression test:

| # | Lens | Sev | Finding | Fix |
|---|------|-----|---------|-----|
| A | auth | **H** | `authorize` was asked about each account's **net** delta and a zero delta is a credit, so `@econ/pay <ops>=<ops> 1` let any player append journal entries and bump `revision` on any scoped wallet | authorize the **gross debit** per account; `econ_pay` refuses `from == to` |
| B | auth | M | the existence-hiding renderer collapsed only against the command's subject; an error naming another resource leaked ("no such account: 4242" vs "account 3 is not active"; `@asset/maintenance` → "scope and owner do not agree") | `EconomyError` exposes every id it names; the renderer falls back to "No such …" if **any** is unviewable |
| C | desync | **H** | `move_to` files an `ObjKind::Exit` into `exits`, which no custody query walks: an exit could be deposited (journaled), was invisible to `holds`/`contents`, unwithdrawable, let `@vault/close` close a non-empty vault, and froze as custody-mismatch at reboot | the container provider and `may_deposit` refuse exits |
| D | desync | L–M | `may_deposit(obj == actor)` passed (co-located, `controls(a, a)`): a player could deposit themself | refuse the actor and actor kinds |
| E | desync | M | invariant 15 checked "is `to` a backing", not "is `to` inside one": standing inside vault B's backing, `@vault/withdraw X=A to=me` landed X in B with no Deposit authority or effect | `destination_ok` walks `to`'s ancestor chain (depth-capped) |
| F | desync | M | `Deposit` had no `is_vault_backing(object)` guard: vault A's chest deposited into vault B let B's withdraw policy extract A's contents | refuse depositing a vault backing |
| G | desync | L | `mint_or_serve_approval` retired the stale request before the cap checks that can refuse | caps first |
| H | boot | **H** | the boot that detects drift latched `frozen` in RAM, healed the drifted docs, and never persisted the latch — the next boot was clean (an E2-era gap) | `frozen` in `meta`, OR'd at boot, `persist_meta` at boot end, `@econ/unfreeze`; asset drift leaves a per-asset trace |
| I | boot | L–M | boot never re-persisted `meta`, so allocator floors lasted one boot; a corrupt/skipped approval doc did not floor the id | persist meta at boot end; floor on every `approval:` key that parses |
| J | boot | M | the new control-doc writes (approval rule, vault/account/asset status, `@account/open`) carried no durable gate | non-strict `gate_on_durable_ack` in each tail |
| K | boot | L | `Account.revision` was not re-derived at boot and moved backwards on a lag heal (weakens `expected_revisions`) | `revision = max(stored, 1 + touches)` |
| L | boot | L | `@account/open|approval`, `@vault/create` worked while the economy was frozen | refused while frozen; recovery verbs stay |
| M | boot | L | `NamedSet` approver arm did not check liveness — a destroyed named approver's approval kept counting | require a live approver |
| N | auth+desync | L–M | `@faction/migrate` opened the org root account (RAM + deferred persist) before `post`; a refusal left a phantom account until reboot | hoist every refusal `post` would hit above the open; if `post` still refuses, report and return `Ok` so the opened account persists |
| O | boot | M | `@faction/migrate` could not converge after a pool edit between runs (same key, different hash → conflict; pool 0 → "no pool"), leaving value both in the wallet and as a rating | the idempotency record is checked first; if it exists, mark + log the residual + report the transaction |

Noted, not changed: a request whose firing-account set grows after minting is unpostable until TTL
(fail-closed); staff cannot credit a `Suspended`/`Closed` org wallet without reactivating it.
Read-verified sound: `post` ordering and the terminal-fallible rule in every command, both-sides
title rules, custody standing, approval consumption/hash binding/four-eyes, reversal
unforgeability, E2-era journals under the E3 binary (digest pin + live retry), latch polarity
across reboot, entry boundedness, key routing, and the module order.
