# E4 implementation plan — holds, escrow, obligations, and contracts

**Status:** Draft 0.5 — reviewed (three passes: adversarial money-safety, code-grounding, design-fit
against docs 08/08a/08b); every confirmed finding is folded below and itemised in §11. The owner's
answers to §10 are folded (Draft 0.3: automatic reconciliation + a graded escalation ladder,
D-E4-14). Draft 0.4 folds two requirements E5's planning fed back (08d §7): **cadence clocks ship in
E4** (D-E4-2 revised) and **canon attestation matches a specific source** (§6.2). Ready for build
once the K-series rebase lands. Companion to
[08-economy-organizations-ownership.md](08-economy-organizations-ownership.md) (the design; §8, §12,
§18-E4), [08a-economy-build-status.md](08a-economy-build-status.md) (the ledger + the E4 pickup), and
[08b-e3-implementation-plan.md](08b-e3-implementation-plan.md) (E3 as built — E4 extends it).
Grounded in the tree at `9d5ea1d`; every `file:line` was checked by the grounding review.
Departures from doc 08 are numbered decisions (D-E4-n, §8).

---

## 1. Scope

From doc 08 §18-E4 and the hooks 08a says E3 left:

1. **Holds** — place / capture / release / expire; `Account.held_amount` becomes a real projection
   and **available balance** (`posted − held`) is what a debit is checked against.
2. **Clocks, obligations, bounded due processing, catch-up, grace** — recurring payments (rent,
   payroll, maintenance) that attempt settlement on a declared clock, never charge a downtime
   backlog, and turn failure into a story state rather than a teleport.
3. **Contracts** — typed parties, terms, conditions, escrow, deterministic splits, settlement,
   disputes, cancellation, expiry. Contracts are **the counterparty-consent carrier** E3 lacked
   (D-E3-4): a two-party sale, a run payment, a lease.
4. **Visibility-safe economic events, canon/evidence projection, and the disclosure filter** for the
   first non-staff journal read — plus the E3 leftovers: approve/deny into the org audit ledger,
   `Encumbrance::Lease`/`Collateral`.

**Exit criterion (doc 08):** scenarios 13–17 pass with duplicate scheduler events, restart, stale
revisions, large time jumps, and hidden parties.

### 1.1 Explicitly out of E4

- **`EconomicClock::Registered { provider, id }`** — doc 08 §18-E4 says "add registered clocks",
  but no clock-provider seam exists anywhere and no consumer needs one before E7's campaign clocks
  (D-E4-2). E4 ships `RealUtc`, `WorldIc`, `Manual`, and economy-owned `Cadence(CadenceId)` records
  (Draft 0.4, for E5's named campaign clock); the enum stays open and becomes an adapter over doc
  09's `ClockRef` when K4 lands (§12).
- **Ruleset consequence policies** (eviction windows, lifestyle strain, loyalty) — E5 hooks onto
  the `ObligationChanged` event; E4 stops at `Grace → Delinquent → Reconciliation` state.
  `Obligation.amount` is a fixed `Amount` per period; lifestyle-tier or percentage amounts are an
  E5 `ObligationAmount` variant (the enum stays open).
- **Work orders / production reservations** (E7), **markets/quotes/fences** (E6/E8), **fee and tax
  templates**, **interest**.
- **Settlement conditions that react to LLM output or narrative prose** — never (§13). And
  conditions that react *synchronously to a `WorldEvent` for money*: no event source is stably
  identified, immutable, and durable at event time — the survey found none, and doc 09's own K2
  status says so of action transactions too ("a K2 compatibility bridge over the current
  process-local serial receipt, not K3's durable token/idempotency protocol",
  `mush-server/src/world_task.rs:2595-2598` on the K branch; no `AuthoritativeOutcome` or durable
  `CommitToken` exists) — so settlement rests on explicit attestations and the economy's own
  journaled facts, re-verified at settlement (D-E4-4). A `FactProvider` over K2 action
  transactions is **not** built in E4; revisit after K3 ships and a ruleset stores immutable
  per-fact records in a state family.
- **Per-obligation or per-contract scheduler entries.** The core scheduler is in-RAM, re-armed from
  scratch at boot, and only `Scene` ticks are dynamic (`mush-core/src/module.rs:208-220`,
  `engine.rs:503-522`); everything runs on one bounded economy sweep (D-E4-3).
- **A per-debtor durable gate from the sweep.** The world step keeps one gate (`engine.rs:1130-1136`,
  first actor wins), so per-notice gating is not expressible; instead the sweep never emits a
  confirmation that claims money moved (D-E4-6), and no core change is needed.
- **A durability checkpoint** (08a decision 12) — still deferred; E4 adds journal volume (holds,
  obligations) and is the point to *measure* boot cost, not to build the checkpoint.
- **Player-created orgs**, **`PolicyRef`**, **per-account isolation of the whole-economy freeze**,
  **widening `EconomicVisibility`** beyond its shipped `Parties | Staff | Public` enum.

---

## 2. Cut structure

Four cuts, each independently CI-green and **each adversarially verified before the next builds
on it** (08a's recommendation after E3's single end review):

| Cut | Crate(s) | One line | Verify axis |
|-----|----------|----------|-------------|
| **E4a** | `mush-economy` (pure) | `Hold`/`HoldOp`/`HoldEffect`; available-balance enforcement; the clock/schedule/catch-up math; `Obligation` + `Contract` records and their pure state machines; the deterministic split; `ContractAuth`; `plan_hash` v3 | double-spend, rounding, catch-up arithmetic |
| **E4b1** | `mush-economy` module | holds + obligations live: persistence, journal-derived `held_amount` with its own seq anchor, the **economy sweep** tick (holds + obligations + GC), IC-clock plumbing, `post_internal`, `@hold/*`, `@obligation/*` | duplicate fires, restart, time jumps |
| **E4b2** | `mush-economy` module | contracts end to end: draft/offer/accept (escrow at accept)/attest/settle/dispute/cancel/expire, live re-verification at settlement, custody locks, `Lease`/`Collateral`, the sweep's contract step, `@contract/*` | consent forgery, settle-twice, hidden parties |
| **E4c** | `mush-economy` + `mush-org` + `mush-jobs` + `mush-server` glue | the `FactProvider` seam (jobs implements it), the `CanonRecorded` attestation observer, `EconomyObserver` events + the canon evidence bridge, approvals → org audit, `@econ/history` under the disclosure filter | existence leaks, evidence leaks |

Dependency directions: `mush-economy → {core, org}` unchanged (it subscribes to core's
`CanonRecorded` event; it never depends on canon). New edges: `mush-jobs → mush-economy` (E1c
direction — the fact provider lives with the data; the grounding review confirmed no cycle:
neither economy nor org depends on jobs, directly or dev), and `mush-server` implements the canon
bridge (the composition root already depends on canon and economy). `mush-scenes` gains nothing: a
scene concludes by committing canon (`mush-scenes/src/lib.rs:282-311`), which the observer sees.
**One `mush-core` change** in E4 (found by the post-K fit review): `WorldEvent::CanonRecorded`
carries only `{ canon_id, kind, entities }` (`mush-core/src/module.rs:523-530`, unchanged by K) and
`mush-canon` exposes no id→event query, so source-specific attestation (§6.2) needs `source: String`
added to that event — a lifecycle-notification field, which doc 09 C6 explicitly keeps `WorldEvent`
for. Its own commit in E4c.

---

## 3. E4a — the pure library

### 3.1 Ids and clocks (`ids.rs`, new `clock.rs`)

`int_id! { HoldId }`, `int_id! { ObligationId }`, `int_id! { ContractId }`.

```rust
pub enum EconomicClock { RealUtc, WorldIc, Manual, Cadence(CadenceId) }   // Registered{provider} still deferred (D-E4-2)
/// A named campaign/domain clock (owner answer 7 to E5): economy-owned, staff/config-defined,
/// advanced by the sweep from its `source` with bounded catch-up (excess recorded as forgiven
/// periods), or by `@econ/cadence advance <id>` for `Manual`. A schedule on a cadence expresses
/// `next_due` as a period index; obligations, holds, contracts, and E5's housing/career
/// evaluation all run on it — never on an IC:OOC ratio or a literal calendar.
pub struct CadenceClock { pub id: CadenceId, pub name: SmolStr, pub source: CadenceSource /* RealUtc | WorldIc | Manual */, pub period: i64, pub catch_up_cap: u8, pub position: u64, pub advanced_at: i64, pub revision: u64 }
pub struct EconomicDeadline { pub clock: EconomicClock, pub at: i64 }
pub struct EconomicSchedule {
    pub clock: EconomicClock,
    pub next_due: i64,                // absolute, on `clock`
    pub cadence: Option<i64>,         // None = one-shot
    pub maximum_catch_up: u8,         // ≤ MAX_CATCH_UP_CAP (8), clamped at creation whatever the caller asked
    pub manual_now: i64,              // the Manual clock's position (advanced only by an explicit act)
}
/// Pure: how many periods are due at `now` (0 when `now < next_due` — a clock that moved backwards
/// never yields a garbage count), and the schedule after settling `n` of them.
pub fn due_periods(s: &EconomicSchedule, now: i64) -> u32;
pub fn advance(s: &EconomicSchedule, n: u32) -> EconomicSchedule;
```

`ValidateEnv` (`mush-economy/src/engine.rs:218-225`, built once at `api.rs:575`) gains `ic_now: i64`
beside `now`; the module feeds `w.now()` and `w.game_now_unix()` (`mush-core/src/world.rs:299-387`).
A `Clocks { real, ic }` helper resolves a deadline or schedule on its clock, `Manual` from the
record's own `manual_now`. Every deadline that belongs to a schedule (grace) is typed
`EconomicDeadline` **on the schedule's clock** — never a bare `i64` compared against the wrong clock.

### 3.2 Holds (`record.rs`, `plan.rs`, `engine.rs`)

```rust
pub struct Hold {
    pub id: HoldId,
    pub account: AccountId,
    pub amount: Amount,                          // > 0
    pub beneficiary: Option<PrincipalId>,
    pub purpose: HoldPurpose,                    // Escrow { contract } | Reservation { kind: SmolStr, id: u64 } | Manual { reason }
    pub placed_by: PrincipalId,
    pub expires: Option<EconomicDeadline>,
    pub status: HoldStatus,                      // Active | Captured { transaction, amount } | Released | Expired
    pub last_journal_seq: u64,
    pub revision: u64,
}
pub enum HoldOp {
    Place   { account: AccountId, amount: Amount, beneficiary: Option<PrincipalId>, purpose: HoldPurpose, expires: Option<EconomicDeadline> },
    Capture { hold: HoldId, amount: Amount },    // explicit: ≤ hold.amount; the remainder is released (invariant 23)
    Release { hold: HoldId },
    Expire  { hold: HoldId },                    // sweep-only (cause Internal { kind: "hold.expire" })
}
```

`EconomyPlan` gains `hold_ops: Vec<HoldOp>` (≤ `MAX_HOLD_OPS_PER_PLAN` 32, ≤ 1 per hold) and
`expected_hold_revisions`. `EconomicTransaction` gains `#[serde(default)] hold_effects:
Vec<HoldEffect { hold: Hold /* full post-state */, kind }>`. `Prepared` carries them. `Account`
gains `#[serde(default)] last_hold_seq: u64` — the journal seq of the last hold effect touching
the account, **distinct from `last_journal_seq`**, which `apply` bumps only for money-line deltas
(`engine.rs` `apply`, the `balance_deltas` loop); an effect-only hold placement never moves
`last_journal_seq`, so `held_amount` needs its own anchor for the drift check (§4.2).

**Holds are journal-authority** (D-E4-1): placing, capturing, releasing, and expiring are effects
inside journal transactions (a `Place`/`Release`/`Expire` is an effect-only entry, legal under E3's
invariant 10); `hold:{id}` docs are pure projections; **`Account.held_amount` is re-derived at boot**
as Σ of the account's `Active` holds and healed/frozen with the same directional rules as
`posted_balance`, anchored on `last_hold_seq`.

### 3.3 Invariants (extending E3's 10–22)

23. **Available balance.** For every account a plan debits, `posted' − held' ≥ floor`, where `held'`
    counts `Place`s in the same plan and **excludes the captured amounts** of this plan's
    `Capture`s (a capture consumes reserved value; the uncaptured remainder of each captured hold
    is released in the same effect). Per-hold attribution is exact because `Capture` carries its
    `amount` (≤ `hold.amount`); several holds on one account may be captured in one plan and each
    effect records its own `captured`. `CreditPolicy` is evaluated against *available*, not posted,
    for every debit in the economy (D-E4-9). **`Place` is refused on an account whose
    `credit.floor` is `None`** (`CreditPolicy::permits` is vacuous there — a reservation on an
    unbounded-credit account reserves nothing) and on any non-`Active` account.
24. `Capture`/`Release`/`Expire` require the hold `Active`; `Expire` requires the deadline passed on
    its clock and is only legal with cause `Internal { kind: "hold.expire" }` (the sweep).
25. Authorization: `Place` needs **debit authority** on the account (a promise to pay —
    `authorize(actor, account, −amount)`); `Capture` needs debit authority on the account **or** is
    an internal settlement whose contract acceptances cover it (`ContractAuth`, §3.5); `Release`
    needs the placer's authority, the beneficiary's (releasing their own claim), or staff. A
    `Place` is a debit-authority plan and is therefore **subject to the account's `ApprovalRule`**
    like any other (D-E4-11).
26. Caps: `MAX_ACTIVE_HOLDS_PER_ACCOUNT` (64); a hold's `amount` ≤ the account's available at
    placement (a hold cannot promise money that is not there).
27. `hold_ops` join the canonical hash under skip-if-empty with a total order
    `(hold-or-account id, discriminant, canonical bytes)`; an E3-shaped plan hashes byte-identically
    (a second hard-coded digest pin beside the E2 one at `hash.rs:138-169`).
28. **`reverse` refuses a target whose `hold_effects` are non-empty** (extending E3's `HasEffects`
    rule): a capture is unwound by a contract refund or an attributed compensating plan, never by
    a reversal that leaves the hold `Captured` while the payee's credit vanishes.
29. **`@account/close` refuses while `held_amount != 0`, any hold on the account is `Active`, or an
    `Active`/`Grace` obligation names the account** (the E3 close path was written when
    `held_amount` was permanently zero; a closed account would trap escrow forever).

### 3.4 Obligations (`obligation.rs`, pure state machine)

```rust
pub struct Obligation {
    pub id: ObligationId,
    pub debtor: PrincipalId, pub creditor: PrincipalId,
    pub debit_account: AccountId, pub credit_account: AccountId,
    pub amount: Amount,                          // fixed per period in E4 (ObligationAmount enum stays open for E5)
    pub schedule: EconomicSchedule,
    pub grace: GracePolicy { pub periods: u8 },  // missed periods tolerated before Delinquent
    pub notify_on_success: bool,                 // default false — routine settlement is silent (§14.3)
    pub status: ObligationStatus,                // Active | Grace { missed, deadline: EconomicDeadline } | Delinquent | Suspended { reason: SuspendReason } | Satisfied | Cancelled
    pub cause: ObligationCause,                  // Contract(ContractId) | Staff { reason } | Ruleset { namespace }
    pub periods_settled: u32,
    pub history: Vec<ObligationEvent>,           // bounded (32): Settled | Missed | Forgiven { periods, from, to, cause } | Escalated { id } | Resumed — the audit tail
    pub revision: u64,
}
pub enum SuspendReason { Staff { reason }, Escalated(EscalationId) }
pub enum ForgiveCause { Downtime, TimeJump, Overload }
/// Pure sweep step for one obligation at `now`: which period indexes to attempt (≤ maximum_catch_up),
/// and the status if more than that are due (→ Reconciliation) — no money logic here.
pub fn plan_sweep(o: &Obligation, now: i64) -> SweepIntent;
/// Pure transition on a settlement outcome (Posted | AlreadyPosted | InsufficientAvailable | Refused).
pub fn advance(o: &Obligation, period: u32, outcome: SettleOutcome, now: i64) -> Obligation;
```

Each period's settlement is an ordinary money plan with `cause: Internal { kind: "obligation", id }`
and **idempotency key `obligation/{id}/{period}`** (the period index is `periods_settled + k`, so a
restart mid-sweep re-derives the same keys and resolves to `AlreadyPosted` — the exit criterion's
"duplicate scheduler events"). A failed period → `Grace { missed: 1, deadline }`; further failures
increment `missed`; past `grace.periods` → `Delinquent` (no further attempts; the E5 consequence
hook takes over — a story state, not a staff page).

**Reconciliation is automatic (D-E4-14; owner decision 2026-09-03).** When more periods are due
than `maximum_catch_up`, the sweep settles the **most recent** `maximum_catch_up` periods and
**forgives** the older excess: the schedule advances past them, and `history` gains
`Forgiven { periods, from, to, cause }` with both debtor and creditor notified once. Doc 08 §2
already forbids charging a downtime or time-jump backlog, so forgiveness is the *designed* outcome,
not a fallback; §8.2's "explicit staff-visible reconciliation state" becomes this attributed
history record plus an `ObligationChanged` event, **not a wait state** — there is no
`Reconciliation` status. The cause is classified from the economy's own persisted sweep stamps
(`meta.last_sweep_real`, `meta.last_sweep_ic`): `Downtime` when no sweep ran across the missed
periods; `TimeJump` when the IC clock advanced more than `real_elapsed × ic_ratio` allows (an epoch
or ratio change); `Overload` when sweeps did run but this obligation never received post budget.
`Overload` forgiveness twice within `ESCALATION_WINDOW` escalates (a misconfigured cadence, or a
starved pool — a human question). **`Delinquent` and `Suspended` remove the obligation from the due
pool** — that, not the per-sweep cap, prevents a long-suspended obligation from starving others.
Changing `cadence`/`next_due` on a live obligation is staff-only and never lowers
`periods_settled`. Obligation status is **doc-authority** (`obligation:{id}` container); its
settlement history is the journal; `history` is the bounded human-readable tail.

### 3.5 Contracts (`contract.rs`, pure)

```rust
pub struct EconomicContract {
    pub id: ContractId,
    pub issuer: PrincipalId,
    pub consent: ConsentSurface {                // EVERYTHING acceptances bind to — hashed as one canonical blob
        parties: BTreeSet<PrincipalId>,          // ≤ MAX_PARTIES (16), includes issuer
        terms: ContractTerms,
        conditions: Vec<TypedCondition>,         // ≤ MAX_CONDITIONS (16)
        deadline: Option<EconomicDeadline>,
        visibility: EconomicVisibility,          // Parties (default) | Staff | Public — parties consent to disclosure
        scope: Option<OrgScopeId>,
    },
    pub consent_hash: [u8; 32],                  // frozen at Offer; any change after that is a new contract
    pub acceptances: Vec<Acceptance { party, at }>,
    pub satisfied: Vec<Option<Attestation { by: AttestorRef, at, source }>>,   // parallel to conditions
    pub escrow: Vec<HoldId>,
    pub state: ContractState,                    // Draft | Offered | Active | ReadyToSettle | Settled { transaction } | Disputed | Cancelled | Expired
    pub revision: u64,
}
pub struct ContractTerms {
    pub consideration: Vec<Consideration { from: PrincipalId, account: AccountId, amount: Amount }>,   // escrowed at accept
    pub deliverables: Vec<Deliverable>,          // ≤ MAX_DELIVERABLES (16): TitleTransfer { object, from, to } | DepositInto { object, vault } | Collateral { object, from } | Obligation { template }
    pub split: Vec<PayoutShare { to: AccountId, share: Share }>,   // ≤ MAX_SHARES (32); Share::Exact(Amount) | Share::Permille(u32)
    pub remainder_to: AccountId,
    pub on_expiry: Expiry,                       // Refund | Forfeit { to: AccountId } | Dispute
}
pub enum TypedCondition {
    Attestation { by: AttestorSet },             // Party(PrincipalId) | Capability(scope) | Staff | Canon { kind, entities_include }
    DepositInto { object: ObjId, vault: VaultId },        // the economy's own journaled custody effect
    TitleHeld  { object: ObjId, by: PrincipalId },       // journal-authority title state
    Fact       { fact: ExternalRef },                     // the E2 `CausalRef { provider, kind, id, revision }`, renamed (§12); via FactProvider (E4c)
    Before     { deadline: EconomicDeadline },           // absolute time
    After      { condition: usize, delay: i64, clock: EconomicClock },   // relative: true once condition[i] has been satisfied for `delay` (E6's fence settlement delay)
}
/// Pure: the settlement plan for a ReadyToSettle contract — captures every escrow hold with an explicit
/// amount, applies the deterministic split (permille floors; remainder to `remainder_to`), the
/// deliverables' title ops, collateral release; cause Internal { kind: "contract", id }, key `contract/{id}/settle`.
pub fn settlement_plan(c: &EconomicContract, snapshot: &EconomySnapshot) -> Result<EconomyPlan, ContractError>;
/// Pure: refund plan (cancel/expiry-refund) — release every escrow hold; key `contract/{id}/refund`.
pub fn refund_plan(c: &EconomicContract) -> EconomyPlan;
/// Pure: feasibility at Offer — split sums, deliverable counts vs MAX_ASSET_OPS/MAX_CUSTODY_OPS, one
/// custody op per settlement (E3's module cap), every referenced account/vault exists.
pub fn check_offerable(c: &EconomicContract, snapshot: &EconomySnapshot) -> Result<(), ContractError>;
```

**Consent (D-E4-4).** `@contract/accept` runs **under the accepting party's live authority** and,
in the same step, places that party's consideration hold (invariant 25 — debit authority, checked
**independently** of the right to accept: an org member needs both `contract.accept` on the
contract's scope *and* debit authority on the org account named in the consideration). An
acceptance is recorded against `consent_hash`; changing anything in the consent surface after
`Offer` is refused (a new contract is drafted). If the consideration hold trips the account's
`ApprovalRule`, the accept returns `NeedsApproval`, **no acceptance is recorded**, the contract
stays `Offered`, and the party re-runs accept once approved (D-E4-11). When every party has
accepted the contract becomes `Active` (doc 08 §8.4's `Accepted` state is folded into `Active`,
D-E4-10). **Settlement authority is the set of acceptances**: `ContractAuth` (an `AuthEval` the
pure engine takes for internal plans with `cause: Internal { kind: "contract", id }`) authorizes
exactly the captures (per hold, per amount), title transfers (both sides — source and destination
parties both accepted, D-E3-12 holds), and collateral releases the consent surface describes, and
nothing else. No authority is stored; the acceptance *is* the authority, bound to the hash and
re-checked at settlement against the live record.

**Conditions** are satisfied only by durable facts and **re-verified live at settlement**
(D-E4-4): an explicit `@contract/attest` by an actor in the `AttestorSet` (a `Party` attestation
stands once given; a `Capability`/`Staff` attestor is re-qualified at settlement like an approver —
E3's A5 rule), a journaled `Deposit` into the named vault (**re-checked at settlement against the
journal-derived last-custody map**, and while an `Active`/`ReadyToSettle` contract's `DepositInto`
condition names an object in a vault, that object is **custody-locked**: only settlement or refund
may withdraw it — `CustodyOp::Withdraw` consults the contract locks), journal-authority title
(re-checked), a `FactProvider` answer re-resolved at settlement (`Some(true)`; `None` = unknown =
unsatisfied), or the clock. **Settlement is a deliberate, idempotent post**: the step that
satisfies the last condition moves the contract to `ReadyToSettle` and settles in the same step if
every re-verification passes; a re-verification failure leaves it `ReadyToSettle` with a loud log
for the sweep to retry (and `Disputed` after `MAX_SETTLE_ATTEMPTS`). Deadline- and `After`-driven
readiness is settled by the sweep. Settling twice is `AlreadyPosted` (scenario 16).

**Deterministic split** (scenario 17): `Exact` shares first, then `Permille` shares of the remaining
escrowed total, floored; the remainder goes to `remainder_to`; feasibility (`Σ Exact ≤ total`,
`Σ Permille ≤ 1000`, deliverable counts within E3's per-plan caps) is checked at `Offer`
(`check_offerable`) and again at settlement.

**E4b2 extensions required by E6's owner answers (08e §4, 2026-09-03)** — E4a's single split is the
degenerate case of each:
- **Outcome-indexed terms:** `ContractTerms.outcomes: Vec<OutcomeTerms { name, when: Vec<usize>
  /* condition indexes */, payouts: Vec<PayoutShare>, expense_treatment, asset_consequences }>`,
  bounded (`MAX_OUTCOMES` 8) and data-driven (`success`/`partial`/`failure` as defaults, never
  hard-coded), each outcome's exact shares summing to its total with **no implicit remainder**;
  separate named slots for payout, arranger fee, expense advance, reimbursable reserve, and
  bonuses/penalties. `settlement_plan(&c, outcome)` builds the plan for the outcome whose `when`
  conditions are satisfied; ambiguity (two outcomes satisfied) is `Disputed`.
- **Roster lock and amendment:** an `Offered` contract may carry slot shares and an unfrozen party;
  the party's acceptance **freezes the roster snapshot and exact shares into the consent hash**;
  a later substitution is an `Amend` that re-hashes the consent surface and requires re-acceptance
  from the affected parties only (others' acceptances stand).
- **Named adjudicator and permitted evidence:** the consent surface names the adjudicator
  (`AttestorSet`) and the permitted condition kinds; attestation from anyone else is refused.
- **Separate roles:** `EconomicContract` records author, payer (the accountable escrow principal),
  visible representative, facilitator, and adjudicator as distinct fields with independent
  visibility — they may coincide, the model must not assume it.
- **Payer authority:** a run funded from an org account passes the ordinary `economy.account.post`
  scope check and `ApprovalRule` at escrow placement; nothing may fund an offer from an
  uncontrolled faucet.

### 3.5a As built (E4a, `29282fe`)

- `ValidateEnv { auth, custody, facts: &dyn FactEval, now, ic_now, next_hold_id }` — `Place` mints
  a `HoldId` at validate time so `apply` stays infallible; **E4b1 must feed `next_hold_id` from the
  journal-derived floor** (today `EconomyStore::peek_next_hold` floors from the snapshot and
  `next_hold` is not yet in `meta`).
- `AuthEval` gains `authorize_hold(actor, &Hold, &HoldOp)` and `authorize_attestor(actor,
  &AttestorSet)`, default `false`; `FactEval { resolve(&ExternalRef) -> Option<bool> }` with
  `NoFacts`; `reverify(&contract, &snapshot, &env)` re-checks every condition kind at settlement.
- `CausalRef` is **hard-renamed `ExternalRef`** (serde shape byte-identical); `pub type CausalRef =
  ExternalRef` kept one release.
- `ConsentSurface.parties` is a `Vec<PrincipalId>` canonicalized by principal key (`PrincipalId`
  has no `Ord`); duplicates refused at `check_offerable`. An `Attestation` records its **clock** so
  `After` compares stamps only on one clock (doc 09 §2.5). `AttestorSet::Canon { kind, source }`.
- Invariant 23 raises **two** errors: `CreditViolation` keeps E2's meaning (`posted' < floor`);
  `InsufficientAvailable` is `posted' − held' < floor`; placement also requires `available' ≥ 0`.
- `Expire` requires `authorize_hold` **and** the sweep cause; the custody unlock requires the
  locking contract's cause **and** one of its two deterministic keys (`cause` alone is
  caller-settable through `post`).
- `Obligation.periods_settled` counts **closed** periods (settled, missed, or forgiven) — it is the
  key-index base, not a success counter; otherwise a miss would re-derive the same
  `obligation/{id}/{period}` key and `grace` would exhaust within one sweep.
- `settlement_plan` accepts `Active` as well as `ReadyToSettle` (the step satisfying the last
  condition settles in the same step); `Deliverable::DepositInto`/`Obligation` emit no plan ops at
  settlement. `Encumbrance::{Lease, Collateral}` exist; `ContractAuth` also authorizes placing the
  contract's own collateral. `EscalationId` and `forgive()` exist.
- One out-of-crate line: `mush-faction/src/econ.rs` `Account` literal gains `last_hold_seq: 0`.
- Digest pins: E2 `9c917eee…d74e` and E3-shaped `b780e3a0…5949`, both green.

**Verify-fold deltas (E4a verify fold).** The lines above describe `29282fe`; the adversarial
verify at the end of the cut changed seven of them, and §3.7 records the whole outcome:

- **`Capture` binds to a debit.** `authorize_hold` is **mandatory** for `Capture` — the plain
  `authorize(actor, account, debit)` fallback is gone. The plan's gross debit on `hold.account`
  must be ≥ Σ its captures there (`EconomyError::CaptureUnbacked(HoldId)`), and the captured
  amount **is** pre-authorized debit authority: the gross-debit pass asks
  `authorize(actor, acct, −(gross_debit − Σ captured))` only when that uncovered part is > 0, and
  the net pass is asked about the same residual. `ContractAuth::authorize` therefore grants **no
  bare debit authority at all** (a negative delta is `false`) — its old "up to what is escrowed on
  that account" ceiling would double-count the escrow now that the captures are excused.
- **The custody lock has no exception.** An object custody-locked by a live (`Active`/
  `ReadyToSettle`) contract's `DepositInto` condition cannot be withdrawn by **any** plan; both
  halves of the old cause+key gate were forgeable from outside (`cause` is caller-settable and the
  two keys are derived from a readable id). The lock lifts on the **state change**;
  `contract::is_contract_key` is deleted.
- **Escrow is qualified, never listed.** A hold in `Contract.escrow` counts only when
  `purpose == HoldPurpose::Escrow { contract: c.id }`, `status == Held`, and `(account, amount)`
  matches a `Consideration` — each consideration claimed by at most one hold (`qualifying_escrow`).
  `settlement_plan` requires every consideration matched (`ContractError::EscrowIncomplete`);
  `refund_plan`, `escrowed_by_account` and `ContractAuth::authorize_hold` see only qualifying holds.
- **Signatures.** `settlement_plan(c, snapshot, env: &ValidateEnv)` calls `reverify` itself;
  `refund_plan(c, snapshot) -> Result<EconomyPlan, ContractError>` requires state ∈ {`Cancelled`,
  `Expired`} (`ContractError::WrongState`). `consent_hash(issuer, consent)` — the issuer is part of
  what a party agreed to. `advance(o, period, outcome, now) -> Result<Obligation,
  ObligationError>`: `period < periods_settled` is a duplicate (`Ok(unchanged)`), `>` is
  `OutOfOrder` (which also enforces forgive-before-attempts), `==` closes the period and records
  `Settled` **whatever the status** — the status itself only moves for an obligation in the due
  pool.
- **Obligations.** A one-shot that misses goes straight to `Delinquent` (there is no next period to
  grace into). `plan_sweep` never saturates into the key index: when `periods_settled + due` or
  `+ cap` would overflow it sets `SweepIntent::reanchor` and forgives wholesale via
  `forgive_all` + `clock::reanchor`, consuming no index. `Obligation` gains
  `settled_total`/`missed_total`/`forgiven_total: u32` (`#[serde(default)]`) so the 32-entry FIFO
  history never loses the tally. `classify_forgiveness` reads a rewound wall clock
  (`last_sweep_real > now_real`) as `TimeJump`, never `Overload`.
- **Clocks.** `advance_cadence` clamps `catch_up_cap` to `1..=MAX_CATCH_UP_CAP` and returns a
  `CadenceSource::Manual` cadence **unchanged** (only `advance_cadence_manual` moves it).
  `check_offerable` refuses `Before`/`After` on `EconomicClock::Manual`
  (`ContractError::UnsupportedClock`) and refuses any **cycle** among `After` anchors;
  `reverify`'s `Before` passes `i64::MAX` as `manual_now`, so an unresolvable deadline reads as
  passed and the condition is unsatisfied.
- **Smaller.** Every hold op requires its account `Active` (`InactiveAccount`); a `Place` that
  mints an id the snapshot already holds is `EconomyError::HoldIdReused`;
  `AttestorRef::Canon { canon, kind, source }` and `AttestorSet::Canon { kind, source: Option<_> }`
  are matched on kind **and** source at `reverify`; `Hold.holder` is documented as an attribution
  label, never authority.

### 3.6 E4a tests (`tests/e4a.rs`, ~45 — built: 66; 83 after the verify fold)

Holds: place/capture/release/expire on hand-built snapshots; available enforcement — two holds
that together exceed available refuse the second (scenario 13 pure half); a debit that ignores a
hold is refused; a capture with an explicit partial amount releases the rest and two captures on
one account attribute per hold; capture exceeding `hold.amount` refused; `Place` on an unbounded
account refused; capture of a non-Active hold refused; expire without a passed deadline refused;
`expected_hold_revisions` stale → refused (the exit criterion's "stale revisions"); caps;
`reverse` refuses a capture-carrying tx. Schedule math: `due_periods` across a downtime gap, an IC
jump, and a **backwards** clock (yields 0); `advance`; `plan_sweep` yields ≤ `maximum_catch_up` and
flags Reconciliation; clamp of `maximum_catch_up`; grace/delinquency transitions with the grace
deadline on the schedule's clock. Contracts: acceptance binds to `consent_hash`; any consent-surface
change after Offer refused (party added, deadline moved, condition added, visibility flipped);
`ContractAuth` authorizes exactly the surface's captures/transfers and refuses an extra line, an
unlisted object, or a capture over the hold's amount; `settlement_plan` split with permille floors +
remainder (scenario 17 pure half); infeasible split or oversized deliverables refused at Offer;
`After` becomes true only `delay` after its anchor; `refund_plan`. Hash v3: E3-shaped plan ==
hard-coded digest; hold-op order independence.

### 3.7 E4a verify outcome

Every finding below was confirmed by a probe against `29282fe` and folded in; each carries a
regression test in `tests/e4a.rs` under `// ---- E4a verify fold ----`, named `vf_<n>_…`.

- **Authorization (critical/high).** `Capture` accepted plain debit authority on the account, so an
  account's owner could spend a reservation standing between them and their own money; and nothing
  tied a capture to a debit, so an effect-only capture destroyed a reservation and a capture on X
  with a debit on Y spent one account's escrow to move another's (finding 1). The custody lock's
  cause+key gate was forgeable from outside (2). A hold merely listed in `Contract.escrow` raised
  the contract's ceiling, was releasable by its refund, and padded its settlement total (3).
- **Allocator (high).** A stale `next_hold_id` re-minted a live hold's id, and `apply` would have
  overwritten the reservation it was standing on (4).
- **Settlement (medium).** `settlement_plan` did not re-verify, so an unmet condition could be paid
  by a caller who forgot the separate `reverify`; `refund_plan` would release escrow in any state
  (6). A canon attestation matched on nothing but "is a canon reference" (5).
- **Clocks (high/medium).** `Before`/`After` on a `Manual` clock read the deadline against a
  position nobody set (10). `advance_cadence` self-advanced a manual cadence and took zero periods
  on a stored cap of 0 (12).
- **Obligations (high/medium).** `advance` dropped a settlement outcome for an obligation out of
  the due pool — real money, silently unrecorded — and folded out-of-order periods (9). A one-shot
  that missed entered a grace whose deadline never expires (11). `plan_sweep` saturated into the
  period index, handing several periods one idempotency key (13).
- **Low.** The consent hash omitted the issuer (14a); `After` anchors could form a cycle nothing
  detected (14b); a rewound sweep stamp classified as `Overload`, which escalates on repeat (14c);
  the 32-entry history FIFO was the only tally of settled/missed/forgiven periods (14d); and
  `hash.rs`'s invariant-27 comment claimed a `Place`-first order the key does not produce (14e).
- **Account status (low).** Hold ops did not check the account's status, so an effect-only release
  or expiry was the one way left to move a frozen account's numbers (7).
- **Attribution (doc).** `Hold.holder` is caller-chosen; nothing may branch on it (8).

**Handoffs.**

- **E4b1** must derive `next_hold_id` from the **journal** (`max(hold_effects.hold.id) + 1`), never
  from surviving docs — `HoldIdReused` is the backstop, not the mechanism. `WorldAuth::
  authorize_hold` now decides *everything* about who may touch a reservation: the **beneficiary**
  captures, the **placer** (or beneficiary, or staff) releases, the **sweep** expires. And
  `Delinquent` has no automatic recovery path: `advance` records a late payment but deliberately
  leaves the status alone, so E4b1 adds a staff/landlord `reinstate` verb or the E5 consequence
  hook owns the transition — until one exists, a delinquent obligation stays out of the due pool.
- **E4b2**'s `refund_plan` runs **after** the state change (which is what lifts the custody lock)
  and, in the same step, returns each deposited object to its depositor with a `CustodyOp::Withdraw`
  authorized by `ContractAuth` — that is the replacement for the deleted lock exception, and
  `ContractAuth` will need a `Deliverable::DepositInto`-derived withdrawal rule to grant it.

---

## 4. E4b1 — holds and obligations live

### 4.1 An internal posting entry

`post` (`api.rs:566-579`) hard-wires `WorldAuth { w: ctx.w }`. E4b1 factors a crate-internal
`post_with(ctx, plan, auth: &dyn AuthEval)` that `post` calls with `WorldAuth`; the sweep and
(E4b2) settlement call it with `WorldAuth` or `ContractAuth`. `post` stays the only public entry.
The sweep is a `TickHandler` (`fn tick`, infallible — `module.rs:224-227`): **every per-hold and
per-obligation outcome is handled by exhaustive match; no `?`, no `unwrap`** — a panic here aborts
the world task, and there is no dispatch-level `Err` to truncate partial work.

### 4.2 Persistence + boot

| Doc | Polarity | Boot rule |
|-----|----------|-----------|
| `hold:{id}` | projection | derived from the journal's last `HoldEffect` per hold; lag heals; disagree-at-claimed-seq / ahead → freeze economy; missing doc → rebuilt silently (no latch to lose); unjournaled doc → not loaded, warn |
| `Account.held_amount` | projection | re-derived = Σ `Active` holds on the account; the stored value is compared **at `last_hold_seq`** (its own anchor — hold effects never move `last_journal_seq`); lag heals, disagreement freezes the economy |
| `obligation:{id}` | container | corrupt → fail boot; debtor/creditor object missing or `GOING`, or either account missing/closed → `Suspended` + re-persist (never delete); a `Contract`-caused obligation whose contract is not `Active`/`Settled` → `Suspended` |
| allocator floors | | `next_hold_id ≥ max(hold_effects.hold.id)+1` **from the journal, never from surviving docs** (GC deletes terminal hold docs; a stale `Contract.escrow` id must never be re-minted); `next_obligation_id ≥` every parsable `obligation:` key; meta high-water marks persisted at boot end (the E3 fold rule) |

`commit_prepared` persists every touched `hold:{id}` projection and the touched accounts (whose
`held_amount`/`last_hold_seq` moved). Latch rule unchanged: a doc-authority write (obligation
status) touches no journal-authority field.

### 4.3 The economy sweep (D-E4-3)

One `TickCadence::Coarse(SWEEP_PERIOD)` tick, tag `economy.sweep` (`Registrar::tick`,
`module.rs:1394-1415`; the handler's `Ctx` has `actor: GOD`, `engine.rs:733-743`, which `post`
never reads for authority). Per fire, in order, **all no-ops while the economy is frozen**:

1. **Expire holds** whose deadline passed on their clock — effect-only posts, cause
   `Internal { kind: "hold.expire", id }`, key `hold/{id}/expire`; ≤ `MAX_HOLD_EXPIRIES_PER_SWEEP`.
2. **Settle due obligations**, oldest `next_due` first among those **in the due pool** (`Active` and
   `Grace` only), ≤ `MAX_OBLIGATIONS_PER_SWEEP` (64) obligations and ≤ `MAX_POSTS_PER_SWEEP` (128)
   posts per fire: `plan_sweep` → attempt each period as a post with key
   `obligation/{id}/{period}`; `Posted`/`AlreadyPosted` → `advance`; insufficient **available**
   (`CreditViolation`) → `Grace`/`Delinquent`; periods beyond the cap → **forgiven** with cause
   (D-E4-14); a refusal only a human can fix (creditor or debtor account closed/missing/frozen, org
   inactive, instrument retired) → **escalate**: `Suspended { Escalated(id) }` for *that* obligation
   and an `Escalation` record (§4.6) — never a whole-economy stop. **Output is exception-driven
   (§14.3, D10): a routine `Posted` is silent unless `notify_on_success`; a transition to `Grace`,
   `Delinquent`, a `Forgiven` record, or `Suspended` notifies debtor and creditor via
   `out.to_actor`.** No sweep notice ever claims that money durably moved (D-E4-6); the whole-step
   gate is not used from the sweep. The sweep stamps `meta.last_sweep_real/ic` at the end of every
   fire (the cause classifier's evidence).
3. *(E4b2 adds)* **Contracts**: expire `Offered`/`Active` contracts past their deadline (`on_expiry`
   → refund / forfeit / dispute); settle `ReadyToSettle` ones whose readiness came from a `Before`/
   `After` condition or whose in-step settlement was deferred by a re-verification failure.
4. **GC** expired approval requests (E3's rule), `Released`/`Expired` holds older than the retention
   window (their history stays in the journal), `Draft` contracts older than `DRAFT_TTL`, and
   acknowledged escalations older than the retention window.

Catch-up is computed from `next_due` and the clock, **never from tick count** — the
`mush-sim/src/route.rs:14-20, 186-208` pattern. The first sweep after a restart fires immediately
(`engine.rs:447-453`) and runs catch-up under the caps; a large IC jump is **forgiven with cause
`TimeJump`**, not charged and not parked; work left over waits for the next fire (§14.3: bounded
per step).

### 4.6 Escalations — the graded ladder (D-E4-14)

The owner's operating constraint: a game has perhaps one staffer who understands an economy alert,
and they may be hours away. So: **resolve programmatically wherever the best action is determinable;
page a human only for a genuine decision; stop future periods only then; never stop the world for a
local problem.** The ladder, used by holds, obligations, and contracts alike:

| Level | Meaning | Economy action |
|-------|---------|----------------|
| 0 — resolve | the best action is determinable | do it, record it (`Forgiven`, a retried refusal, a unanimous party agreement), notify the affected principals once |
| 1 — inform | worth a human's eventual glance, nothing blocked | `EconomicEvent` + a line in `@econ/alerts`; no state stops |
| 2 — page | a human must decide | open an `Escalation { id, kind, subject, context, recommended: Vec<Command>, opened_at, acked_by }` (`escalation:{id}` leaf doc), suspend **only the subject** (`Suspended { Escalated(id) }` / contract `Disputed`), emit `EscalationOpened`; the server glue (E4c) broadcasts to the staff channel and may open a job |
| 3 — stop | the journal cannot be trusted | the existing whole-economy freeze — unchanged — but it now also opens a level-2 record carrying the diagnosis (which doc drifted, at which seq) and the recovery command |

Triggers: level 2 for a settlement refusal only a human can fix (closed/missing/frozen account,
inactive org, retired instrument, a `Settled` contract whose transaction is reversed), for
`Overload` forgiveness twice in `ESCALATION_WINDOW`, for a contract `Disputed` longer than
`DISPUTE_TTL` without unanimous party agreement, and for `MAX_SETTLE_ATTEMPTS` exhausted. Every
record carries **the recommended command(s)** with arguments filled in (`@obligation/repoint #12
creditor=#7`, `@obligation/cancel #12`, `@contract/resolve #4=refund`) — the staffer is going to
paste the alert into an LLM anyway, so the alert should already be the answer. `@econ/alerts`
lists open escalations newest first (staff, capped); `@econ/ack <id>` marks one handled and resumes
the subject where that is the recommendation. Unacknowledged level-2 escalations re-notify at
`ESCALATION_RENOTIFY` intervals (bounded, then daily). Escalation docs are leaves (corrupt → skip
loudly); ids are floored like approvals.

### 4.4 Commands

```text
@hold/place <account>=<amount> [for=<principal>] [expires=<secs|ic:<secs>>] [reason]   (debit authority; ApprovalRule applies)
@hold/release <id>    @hold/capture <id>=<to account> <amount>    @hold/show <id>   @hold/list [account=]
@obligation/create <debtor acct>=<creditor acct> <amount> every=<secs|ic:<secs>> [start=] [grace=<n>] [catchup=<n>] [notify]   (staff / ruleset)
@obligation/show|list|suspend|resume|cancel <id>   @obligation/advance <id>  (Manual clock)   @obligation/repoint <id>=<debit|credit acct>  (staff)
@econ/alerts   @econ/ack <id>                    (staff — the escalation ladder, §4.6)
@account/close  — now refuses per invariant 29
```

All money through `post`; every refusal through the E3 existence-hiding renderer with new
`NamedResource::{Hold, Obligation}` variants and `may_view_hold/obligation` (view = account view on
either side, or beneficiary/party, or staff). `@hold/list account=` follows E3's list discipline:
empty for a viewable-but-empty account, identical "No such account." for nonexistent and
unviewable alike.

### 4.5 E4b1 tests (`tests/e4b1.rs`, ~28 — built: 33; 61 after the verify fold)

**13** two holds racing the same available refuse the second through the live seam; a payment that
would dip into held funds is refused; **14** capture, release, and expiry each: `fold_kv` + `reboot`
→ identical `held_amount` and hold state; `fire_due` twice for the same expiry → one effect; a
restart between the sweep's post and its doc write converges; **15** an obligation settles on
schedule (set_now steps) *silently*, fails into `Grace` with a notice when available is short,
reaches `Delinquent` after `grace.periods`, and after a simulated downtime of ten periods with
`maximum_catch_up = 3` settles exactly the three most recent, **forgives seven with cause
`Downtime`**, notifies once, and stays `Active` on schedule; an IC-ratio jump forgives with cause
`TimeJump`; a starved pool forgives with `Overload` and the second time within the window
escalates; a backwards clock charges nothing; `fire_due` twice at the same instant posts once per
period; the whole-economy freeze stops the sweep and opens a level-2 record; a creditor account
closed under a live obligation → `Suspended { Escalated }` with a recommended `@obligation/repoint`
in the record, `@econ/ack` resumes it after repointing; `@account/close` refused with an Active
hold; `reverse` refused on a capture. Plus the boot quartet for `hold:{id}`, the `held_amount` drift
freeze anchored on `last_hold_seq`, the `next_hold_id` floor after GC, escalation-id floors, and the
hidden-party renderings for holds and obligations.

### 4.7 As built (E4b1)

The cut wires E4a into the running module: an internal posting entry, four new
persisted record kinds and their boot rules, the one economy sweep, the escalation
ladder, `WorldAuth::authorize_hold`, and the `@hold`/`@obligation`/`@econ/alerts|ack`
surface. `tests/e4b1.rs` is **33** tests; `mush-economy` is 280 in total, and both
digest pins (E2 `9c917eee…`, E3-shaped `b780e3a0…`) are green.

**Signatures and new files.**

- `api::post_with(ctx, plan, auth: PostAuth<'_>)` is the crate-internal entry; `post`
  is `post_with(ctx, plan, PostAuth::World)` and stays the only public one.
  `PostAuth::{World, Injected(&dyn AuthEval)}` replaces the plan's bare
  `auth: &dyn AuthEval` for one borrow-checker reason worth recording: `WorldAuth`
  holds `&World`, and a caller already holding `&mut Ctx` cannot construct one to pass
  in. The *choice* is passed instead and the bridge is built inside `post_with`, where
  the borrow lives exactly as long as validation. E4b2's `ContractAuth` goes in as
  `Injected` over a locally-cloned contract.
- **`install_commit` is infallible** (§12): `fn install_commit(ctx, commit) ->
  TransactionId`. Its two post-move checks are asserted invariants carrying the
  reachability argument (the allocator and the journal tail each have exactly one
  mutator, this function; the only call between `prepare_commit` and it is
  `run_custody_effects`, whose providers do not re-enter and whose `CarryAttempt` hook
  chain has no `Ctx`). The custody move remains the last fallible act.
  `tests/e4b1.rs::a_provider_that_re_enters_post_trips_the_install_invariant` drives a
  stub `CustodyProvider` that posts from inside `move_into` and proves the **assertion**
  fires, not a `JournalUnavailable` desync after the world has already moved.
- New modules: `src/sweep.rs` (the tick) and `src/escalation.rs` (the ladder's record).
- `WorldAuth::authorize_hold`: `Capture` → the actor **acts for the beneficiary**
  (object: itself, `perm::controls`, or staff; organization: `economy.account.post` on
  its root compartment, or staff — no new capability was minted, because "may spend the
  org's money" is exactly the right to turn its escrow into a payment; system:
  never from a command, and a beneficiary-less hold is capturable by nobody).
  `Release` → the placer, the beneficiary, debit authority on the account, or staff.
  `Expire` → `api::sweep_principal()` (`sys:economy.sweep`) only, with the engine's
  cause gate unchanged. `Place` fails closed here (the engine asks it as debit
  authority through `authorize`). `Hold.holder` is never read.
- `EconomyStore` gains `next_obligation`/`next_cadence`/`next_escalation`,
  `escalations`, `hold_terminal_at` (RAM-only GC evidence, journal-derived), and
  `last_sweep_real`/`last_sweep_ic`; `from_parts` now takes a `BootParts` struct.
  `index_transaction` advances `next_hold` past every `Placed` effect and stamps the
  terminal instant of every other one.

**Doc shapes.** `hold:{id}` (projection), `obligation:{id}` (container),
`cadence:{id}` (container), `escalation:{id}` (leaf). `meta` gains `next_hold_id`,
`next_obligation_id`, `next_cadence_id`, `next_escalation_id`, `last_sweep_real`,
`last_sweep_ic`. `Escalation { id, level, kind, subject, context, recommended,
opened_at, acked_by, acked_at, notified_at, notifications, revision }` with
`EscalationKind::{SettlementRefused, RepeatedOverload, KeyConflict, EconomyFrozen,
BootReconciliation}` and `EscalationSubject::{Obligation, Hold, Contract, Economy}`.
No record type changed its journalled bytes, so every E2/E3/E4a journal still replays.

**Boot, as implemented.** `hold:{id}` lags → heals silently; disagrees at its own
claimed seq, or is ahead → freeze; missing **or unreadable** → rebuilt silently from
the journal (a hold has no doc-authority field at all, so an unreadable one costs
nothing — this is the one place the plan's table said only "missing"); unjournaled →
warn, not loaded. `Account.held_amount` is Σ the account's `Held` holds, compared at
`last_hold_seq` and healed/frozen with the same directional rules. `obligation:` and
`cadence:` corrupt → fail boot; an obligation whose debtor/creditor object is missing
or `GOING`, whose account is missing or closed, or whose `Contract` cause is not live
→ `Suspended` + re-persist, **never deleted**, and (per D-E4-14, capped at
`MAX_BOOT_ESCALATIONS` = 16) paged with the fix written out. `escalation:` corrupt →
skip loudly, id still floors. Floors: `next_hold` from `max(hold_effects.hold.id)+1`
**and `meta` only** — never from surviving docs; `next_obligation`/`next_cadence`/
`next_escalation` from every parsable key plus `meta`. `meta` is persisted at the end
of every boot, as E3 established.

**The sweep.** `TickCadence::Coarse(30 s)`, tag `economy.sweep`, actor `GOD` (never
read for authority; the *plan's* actor is `sys:economy.sweep`). Order per fire:
cadence clocks → hold expiries (≤ 64, key `hold/{id}/expire`) → obligations (≤ 64
records, ≤ 128 posts, key `obligation/{id}/{period}`, forgive-then-attempt) → GC
(expired approvals, terminal holds and acked alerts past 7 d) → escalation
re-notification → the sweep stamps. `sweep.rs` contains no `?`, no `unwrap`, no
`expect`, and no panicking index; `sweep::disposition` matches **every**
`EconomyError` variant by name with no `_` arm, so a new refusal has to be classified
deliberately as `Short` (grace), `Transient` (retry), or `Human` (page). `advance`'s
`OutOfOrder` and a duplicate outcome are surfaced through a loud log, never a panic.

**Deviations from §4, and why.**

1. **A cadence step was added to the sweep** (D-E4-2 says cadence clocks are "advanced
   by the sweep"; §4.3's numbered list predates that). Manual cadences are untouched,
   which `advance_cadence` also enforces. No `@econ/cadence` command ships — §4.4 does
   not list one and E5 owns the consumer — so in E4b1 cadences exist only through
   persistence, boot and the sweep.
2. **The frozen sweep does not stamp `meta.last_sweep_*`.** §4.3 says "every fire";
   a fire that did nothing did not run, and stamping would let a long freeze classify
   as `Overload` (which escalates on repeat) instead of `Downtime`.
3. **The freeze's companion record is level 2, the freeze itself is the level-3
   action** — §4.6's table read literally. One record per open subject, so a refusal
   that repeats every 30 s produces one alert, not a thousand.
4. **`@econ/ack` does not clear the whole-economy freeze**, and says so: that latch is
   `@econ/unfreeze`'s, deliberately.
5. **Escalation re-notification is a loud log plus a stamp**, not a broadcast: §4.6
   assigns the staff-channel fan-out to E4c's server glue, and this crate has no way
   to reach staff. `@econ/alerts` is the pull side and is live now.
6. **`@obligation/create` refuses a `System` debit account** (a standing mint), and
   `ObligationAuth` refuses it again at every settlement. E5's faucet-funded income
   needs an explicit decision before that opens.
7. **Commands added beyond §4.4's list:** `@obligation/reinstate` (the brief's own
   addition — E4a leaves `Delinquent` with no automatic recovery) and
   `@econ/alerts <id>`, which prints one record's recommended commands.
8. **Syntax made concrete** where §4.4 was shorthand: `@obligation/repoint <id>=<debit|
   credit> <account>`; `every=<secs|ic:<secs>|manual:<secs>>`; `expires=`/`start=` are
   durations **from now** on the clock they name.
9. **Hold ids start at 1**, not 0: E4a's `EconomySnapshot::next_hold_floor` returns 1
   for an empty snapshot and `peek_next_hold` takes the max. Harmless (the journal
   floor is the real allocator now) and left alone rather than perturbing a verified
   pure surface.

**Line-number drift.** Every `file:line` in §4 predates the K merge and none of them
survived it: `post` is `api.rs:569-591` (not 566-579); `ValidateEnv` is
`engine.rs:290-308` (not 218-225) and is built at `api.rs:578`; `TickHandler` is
`mush-core/src/module.rs:230-236` and `Registrar::tick` is `:1986-2006` (not
`module.rs:224-227` / `:1394-1415`); the tick `Ctx { actor: GOD }` is
`mush-core/src/engine.rs:996-1005` (not `:733-743`). One claim is **wrong**, not just
moved: §4.3's "the first sweep after a restart fires immediately
(`engine.rs:447-453`)" — `Engine::arm_registered_periodic_ticks`
(`mush-core/src/engine.rs:667-688`) arms at `now + period`, so the first fire is one
sweep period after boot. Nothing depends on it, because catch-up is computed from
`next_due` and never from tick count.

**Handoffs.**

- **E4b2** posts settlement through `post_with(ctx, plan, PostAuth::Injected(&auth))`
  with a `ContractAuth` built over a cloned contract (the same borrow constraint that
  shaped `PostAuth`). `contract:{id}` boot, the custody-lock re-derivation, and the
  sweep's contract step slot in beside the existing ones; `EscalationSubject::Contract`
  and `EscalationKind::{SettlementRefused, KeyConflict}` already exist for the dispute
  and `MAX_SETTLE_ATTEMPTS` triggers, and `sweep::page` is the one way to open a
  record. A contract-caused obligation is already suspended at boot when its contract
  is not live.
- **E4c** takes the notification seam: `sweep::notify_principal` reaches only
  `PrincipalId::Object`, so an organization or system party is told nothing today —
  the `EconomyObserver` events and the staff-channel broadcast are where that lands,
  along with the escalation re-notification fan-out.
- **E5** owns `@econ/cadence` (creating and manually advancing cadence clocks), the
  `Delinquent` consequence hook now that `@obligation/reinstate` exists as the manual
  path, and the faucet-funded-income decision in deviation 6 — **answered by M1**
  below: `create_system_obligation` plus the `[economy]` mint cap.

**Verify-fold deltas (E4b1's end-of-build adversarial verify).** The as-built above
describes the cut as it shipped; these are the rules that changed when its three
lenses were folded back in. §4.8 records the outcome.

- **Boot floors saturate, and allocators refuse at the ceiling.** `next_after` is
  `saturating_add`, so one key at `u64::MAX` cannot panic the boot or wrap a floor to
  zero; every `EconomyStore::alloc_*` returns `Option` and a caller at the ceiling
  logs loudly and refuses that one operation. A `sweep::page` that cannot mint still
  suspends its subject, with a `Suspended { Staff }` reason.
- **`revision` is journal authority over lines ∪ hold effects.** Replay counts a hold
  effect as a touch exactly as `apply` does, so a place/release run cannot rewind an
  account's revision at boot and an `expected_revisions` guard stays `Stale` across a
  restart. `on_obj_destroyed` no longer bumps `revision` at all — a doc-authority
  status write never moves a journal-authority field (the asset arm was already the
  model).
- **Boot's directional checks have no zero-shaped blind spot.** An account doc that
  claims a `last_journal_seq` or `last_hold_seq` it never appears at freezes, the way
  the asset and hold projections already did; `unwrap_or(0)` could not tell "no effect
  there" from "nothing reserved there".
- **Recovery is scoped and remembered.** Boot records the accounts *its own* latch
  froze in `meta.boot_frozen`, and `@econ/unfreeze` restores exactly those — an
  account frozen for its own reason stays frozen and stays visible.
- **Boot stamps `meta.last_boot_real`.** `classify_forgiveness` takes "the gap spans a
  boot" and calls that `Downtime` at any length; `SWEEP_GAP_DOWNTIME_SECS` drops to
  `2 × SWEEP_PERIOD_SECS` for gaps with no boot marker. A 90-second restart used to
  classify `Overload`, which escalates on repeat.
- **An unresolvable clock is a decision, not silence.** A schedule on a `Cadence(id)`
  with no record is suspended and paged (at boot and in the sweep) instead of sitting
  `Active` and never due.
- **The ladder dedupes by subject *and* by blamed account.** One open record per
  subject; an **acknowledged** record whose cause persists is *reopened* — same id,
  a bounded `reopened` tally — after `ESCALATION_RENOTIFY_SECS`, never re-minted; and
  one blocked account carries one record per `ESCALATION_WINDOW_SECS`, naming the
  other subjects it stopped in its `context`. The whole-economy freeze opens one
  record per freeze, acked or not. `Escalation` gains `blamed` and `reopened`, both
  `#[serde(default)]`.
- **The sweep obeys the org-liveness rules every manual payment obeys.**
  `ObligationAuth` carries the `api::scope_coherent` (debit) / `api::credit_allowed`
  (credit) verdicts, computed immediately before the post; `post_period` asks first so
  the refusal can name the blamed account. A wound-down division stops collecting.
- **Per-fire caps bound work, not iteration.** The cadence step filters on
  `cadence_elapsed(..) > 0` *before* `take(MAX_CADENCES_PER_SWEEP)`.
- **Holds and transient defers have ladders.** Three consecutive refused expiries page
  `EscalationSubject::Hold`; sixteen consecutive `Transient` defers page the
  obligation. Both runs are RAM-only counters on `EconomyStore`, read through
  `expiry_refusals` / `deferred_fires`.
- **`@obligation/resume` delivers what it promises.** It refuses a
  `SuspendReason::Escalated` suspension (pointing at `@econ/ack <id>`, the one verb
  that answers an alert *and* resumes its subject), and for a staff suspension it
  re-anchors the schedule and records the gap as `Forgiven` with the new
  `ForgiveCause::Suspension`.
- **Bounds that were not bounds.** `grace` is capped at `MAX_GRACE_PERIODS` = 254
  (255 was infinite grace, because `missed` saturates there); `expires=` must be
  strictly positive; `@obligation/advance` moves a manual clock at most to `next_due`
  and is a no-op with a notice while a period is due and unsettled;
  `@account/close` is blocked by any obligation that is not `Cancelled`/`Satisfied`,
  and `@obligation/reinstate` re-checks that both accounts are `Active`.
- **Reservations are promises, and are read through one predicate.** Plain debit
  authority releases only a **beneficiary-less** hold; `may_view_hold` asks
  `WorldAuth::acts_for` for the beneficiary clause, so an org's spender and a Thing's
  controller can see (and therefore capture) escrow made out to them. Nobody acts for
  a `System` principal, **staff included** — that arm sits above the staff
  short-circuit — and `@hold/place for=sys:<name>` is staff-only and refused unless
  the principal owns a registered account, with the ordinary unknown-principal text.
- **Narrowed surface.** `EconomyStore::journal`/`frozen` are `pub(crate)` with read
  accessors (`journal()`, `is_frozen()`), which is what makes `install_commit`'s
  "one mutator" argument a property of the type; `api::sweep_principal` is
  `pub(crate)`. `install_commit`'s reachability comment now names the
  `carry_attempt(&mut World, …)` hook chain and why a `&mut World` without a `Ctx`
  cannot post.
- **M1 — faucet-funded obligations (owner decision).** `@obligation/create` keeps
  refusing a `System` debit. The one route is
  `mush_economy::create_system_obligation(ctx, faucet, ObligationSpec)`: the faucet
  must own the debit account (so it is registered), both sides must be `Active` and
  share an instrument, and the per-period amount must sit inside
  `EconomyConfig::mint_cap` — `EconomyError::MintCapExceeded` otherwise. The record
  stores `minted_by: Some(PrincipalId::System(..))` (`#[serde(default)]`), and
  `ObligationAuth` permits a `System` debit **only** for such a record and re-reads
  the cap at every settlement, so a doc edited above it afterwards is paged rather
  than paid. `EconomyConfig { mint_obligation_cap_major: 50_000, instruments }` is
  wired through `mush-config`'s `[economy]` section →
  `mush_server::config_ext::economy_config` → `EconomyModule::with_config`; an absent
  section is the crate default.

### 4.8 E4b1 verify outcome

Three adversarial lenses over the shipped cut, plus one owner decision. **25
findings folded**, `tests/e4b1.rs` goes 33 → 61, `mush-economy` 280 → 309, the
workspace 2 389, both digest pins green.

- **Durability and boot (B1–B8).** Two HIGH: a boot floor that panicked (or wrapped)
  on a key at `u64::MAX`, and five allocators that overflowed — one of them inside the
  tick, where a panic aborts the world. Two MEDIUM: `revision` rewinding for hold-only
  entries (which silently un-`Stale`d an optimistic guard across a restart), and
  freeze recovery that could only be all-or-nothing. One MEDIUM more: a lease pointing
  at a deleted cadence clock sat `Active` and never due. Two LOW: `held_amount`'s
  anchor check going blind at zero, and an obligation blocked by a frozen account
  being told to repoint away from its own money. One INFO: the destruction backstop
  bumping a journal-authority field.
- **The sweep and the ladder (S1–S9).** One MEDIUM-HIGH: the sweep was the only payer
  in the world that org status did not bind, so a suspended division kept collecting
  rent. Three MEDIUM: cadence #65 starving behind 64 no-ops, a 60–120 s restart
  classifying as `Overload` (which escalates on repeat), and `@econ/ack` re-minting a
  fresh alert id every thirty seconds for one unfixed cause. The rest bound what was
  unbounded — hold expiries, transient defers, `grace = 255` — or made `resume`'s
  notice true.
- **Authority and disclosure (A1–A8).** Two HIGH: plain debit authority destroying a
  third party's escrow (a promise only the payer could break), and `may_view_hold`
  hiding a reservation from the very principals who were entitled to capture it —
  an org's spender, a Thing's controller. Two MEDIUM: staff acting for a `System`
  beneficiary, and `@obligation/advance` multiplying a manual clock without bound.
  Invariant 29 was widened to the whole obligation lifecycle. The rest are LOW: an
  escalated suspension lifted silently, a non-positive `expires=`, stale help.
- **M1 (owner).** E5's income table needs a standing mint; a standing mint needs a
  bound and an authority that is not a staff verb. Both now exist.

**Handoffs.**

- **E5** owns `@econ/cadence`, and must test B6 and S2 against **real** cadence
  clocks: E4b1 has no command that creates one, so both are pinned here through
  injected `cadence:{id}` docs. E5's income table is `create_system_obligation`'s
  first caller (08d §5.1).
- **E4b2** must gate `@hold/release` on `HoldPurpose::Escrow`: a contract's escrow is
  released by the contract's own refund path and by nothing else, so the placer clause
  A1 leaves open has to close when contracts start placing holds.
- **Documented, not fixed:** an `ApprovalRule` is a per-transaction escalation and
  every obligation period is its own transaction, so a per-period amount **under** the
  threshold never escalates however long it runs (the sub-threshold drip) and one
  **over** it cannot settle at all. `@obligation/create` says so at creation. A budget
  primitive is E5's.
- **Not reachable from the integration harness:** S7's *increment* path needs a
  `Disposition::Transient`, and both refusals classified transient
  (`JournalUnavailable`, `EconomyFrozen`) are unreachable through `TestWorld` — the
  ledger refuses only on overflows boot itself rejects, and a frozen economy returns
  before the sweep reaches an obligation. The run's bookkeeping is unit-tested in
  `sweep.rs::tests::vf_s7_a_transient_defer_run_is_bounded_and_resets`.


---

## 5. E4b2 — contracts live

Persistence: `contract:{id}` **container** docs (corrupt → fail boot); boot reconciles each
non-terminal contract against the journal: an `Active` contract whose escrow hold is not `Active`
in the journal → `Disputed` + loud log (§15.2 "disable and quarantine, reserve no value");
`Settled { transaction }` whose transaction is **absent** from the journal → freeze the economy (a
settlement the journal does not know is drift); whose transaction is **reversed** → `Disputed` +
loud log (unreachable while invariant 28 holds, checked anyway). `next_contract_id` floored on
every parsable key + meta. Custody locks are re-derived at boot from `Active`/`ReadyToSettle`
contracts' `DepositInto` conditions.

Commands (party-gated unless noted; org contracts also accept `contract.*` capabilities on the
contract's scope — new vocabulary entries `contract.draft`, `contract.accept`, `contract.attest`,
`contract.dispute`, added to `mush_economy::CAPABILITIES` and thus to the server's org vocabulary):

```text
@contract/draft <name> [scope=]            → Draft (issuer); then consent-surface builders (Draft only):
@contract/party <id>=<principal>   @contract/pay <id>=<from acct> <amount>   @contract/deliver <id>=title <obj> <from>→<to> | deposit <obj> <vault> | collateral <obj> <from>
@contract/split <id>=<acct> <exact|N‰> …  @contract/remainder <id>=<acct>   @contract/condition <id>=attest <by…> | deposit <obj> <vault> | title <obj> <by> | fact <provider> <kind> <n> | before <deadline> | after <n> <delay>
@contract/deadline <id>=<deadline> [refund|forfeit=<acct>|dispute]   @contract/visibility <id>=parties|public
@contract/offer <id>                       Draft → Offered (check_offerable; freezes consent_hash)
@contract/accept <id>                      a party; places that party's consideration hold under their live authority
@contract/attest <id>=<n>                  an attestor; may settle in the same step
@contract/dispute <id>   @contract/agree <id>=settle|refund   (a party; unanimous agreement resolves a dispute without staff — level 0)
@contract/cancel <id>    @contract/resolve <id>=settle|refund   (staff — level 2, after DISPUTE_TTL escalates)
@contract/show <id>   @contract/list
```

Caps: `MAX_OPEN_CONTRACTS_PER_ISSUER` (16 non-terminal), `MAX_LIVE_CONTRACTS_PER_PARTY` (32),
`@contract/list` capped at 20, `Draft` GC after `DRAFT_TTL` (7 days).

`Encumbrance::Lease { lessor, contract }` / `Collateral { contract }`: placed at activation for
`Collateral`/`Obligation` deliverables (the source party's acceptance authorizes it), released at
settlement/refund. A lease is a contract whose deliverable is an `Obligation` template: activation
creates the obligation (`cause: Contract(id)`) and the `Lease` encumbrance; cancellation/expiry
cancels the obligation. `Fact` conditions are accepted at draft in E4b2 but no provider exists
until E4c, so they resolve `None` (unsatisfied) — E4b2's tests do not exercise them.

Tests (`tests/e4b2.rs`, ~28 — built: **37**): **16** a Shadowrun run — Johnson drafts, three runners + Johnson
accept (Johnson's escrow hold placed at his accept), the run's condition is attested, settlement
posts the split once; a second attest / a second settle is a HIT; the split's remainder lands where
declared (**17** live half); a party who did not accept cannot be debited; an org member with
`contract.accept` but no debit authority cannot escrow org funds; consent-surface mutation after
Offer refused; settlement refuses an extra line (`ContractAuth`); a sale — buyer pays, seller's
title transfers, both accepted, both authorities re-checked live (revoke the seller's title before
settlement → `Disputed`, nothing moves); **deposit-then-withdraw** — the object cannot be withdrawn
while locked, and a lock-free re-verification failure leaves the contract `ReadyToSettle` with
nothing moved; an `After` condition delays settlement until the sweep; a second attest racing
settlement (stale `revision`) is refused; expiry → refund / forfeit; dispute freezes escrow; a
`Public` contract requires every party to accept that visibility; reboot at every state; **hidden
parties**: a non-party's `@contract/show`, `@contract/accept`, and every refusal render the
nonexistent-id text; a party sees the other parties only as the consent surface names them, never
their balances.

### 5.1 As built (E4b2)

The cut wires E4a's pure consent carrier into the running module: `contract:{id}`
persistence and its boot reconciliation, the `@contract/*` surface, the escrow
placement that rides an acceptance, live settlement, unwind and forfeit, the
`Lease`/`Collateral` encumbrances and the lease's obligation, the sweep's contract
step, and the E6-driven extensions (outcome table, roster lock + amendment, named
adjudicator + permitted evidence, separate roles). `tests/e4b2.rs` is **37** tests;
`mush-economy` is **346** in total, and both digest pins (E2 `9c917eee…`, E3-shaped
`b780e3a0…`) are green. (Those counts are the cut as built; the **Verify-fold deltas**
block below and §5.2 record what the end-of-build adversarial verify changed, and the
totals after it.)

**Signatures and new files.**

- New module `src/contract_live.rs` — `may_view_contract`, the `@contract` command
  surface, and the live machinery the sweep shares with it (`activate`,
  `promote_and_settle`, `settle`, `unwind`, `expire`, `dispute_with`). It contains
  **no `unwrap`, no `expect`, no `panic!` and no panicking index**, because the sweep
  drives the same functions and a panic in a tick handler aborts the world.
- `contract.rs` gains `readiness(c, snapshot, env) -> Readiness::{Ready(outcome),
  NotYet, Undecidable(e)}` — the **one** verdict the command path and the sweep both
  read, so they can never disagree about what a contract is waiting for.
  `conditions_met` is now `matches!(.., Ready(_))`. The third arm is load-bearing: a
  contract that cannot settle *at all* as it stands (two outcomes satisfied at once, a
  satisfaction vector that does not match the conditions, a mutated surface) read as
  "not yet" would freeze its escrow forever while waiting for a condition that is
  already true. `Undecidable` disputes and pages at once instead of spending
  `MAX_SETTLE_ATTEMPTS` on a retry that cannot help.
- `contract.rs` also gains `forfeit_plan(c, snapshot)` + `forfeit_key`,
  `activation_asset_ops(c)` + `activate_key(c)`, `contract_encumbrance(c, from)`,
  `release_asset_ops(c, snapshot)` (only the pledges the asset really carries — a
  contract cancelled while it was still `Offered` never activated, and a `Release` of
  an encumbrance that is not there would fail the whole refund and strand the escrow),
  `resolve_outcome(c, snapshot, env)`, `outcome_conditions(c)`,
  `condition_holds(c, i, snapshot, env)`, `apply_outcome(total, payouts)` and
  `canonical_visibility(v)`. `refund_plan(c, snapshot)` keeps its E4a signature and
  now also emits the pledge releases and the `CustodyOp::Withdraw` that returns each
  recorded deposit; its `actor` becomes the **depositor** when it carries a
  withdrawal, because invariant 15's destination rule is "the actor, or the actor's
  location".
- `ContractAuth` is bounded by an explicit `ContractPurpose` —
  `settling(c, snapshot, outcome)` (`new` is `settling(.., None)`),
  `forfeiting(c, snapshot, to)`, `unwinding(c, snapshot)`. The full table is in the
  type's own doc comment; the short version is that **a settlement captures and never
  releases, an unwind releases and never captures, neither ever grants a debit, and
  the only vault op any of them authorizes is an unwind returning a recorded deposit
  to its depositor**.
- `WorldAuth` gains `authorize_attestor` (E4b1 left it at the trait default `false`,
  which would have made a `Capability`/`Staff` attestation impossible to re-qualify
  and therefore impossible to settle on) and refuses **every** op on a hold whose
  purpose is `Escrow { contract }` — the §4.8 handoff, closed at the engine seam
  rather than at the command.
- `EconomyStore` gains `next_contract`/`alloc_contract` and `last_custody:
  BTreeMap<ObjId, CustodyMark>` (RAM-only, journal-derived, like `hold_terminal_at`);
  `api::contract_key`, `api::put_contract`, `api::contract_principal()`.
- **The deterministic keys carry a generation** (`contract_op_key(id, verb,
  generation)`, `generation = c.amendments`). Generation 0 is byte-identical to E4a's
  `contract/{id}/{verb}`, so every stored idempotency record keeps resolving. It
  exists because an amendment genuinely re-opens a contract: the same id refunds when
  it is amended, is funded again, and can be cancelled again — and a fixed
  `contract/{id}/refund` would resolve to the *first* refund as an idempotency HIT and
  hand nothing back, stranding the second escrow silently. `MAX_AMENDMENTS` (32)
  bounds the generations one id can own.
- `evaluate_approvals` now adds back what a plan **captures**, symmetrically with the
  placement subtraction D-E4-11 introduced: escrow goes through the payer's
  `ApprovalRule` at `@contract/accept`, and asking again at settlement would demand a
  second approval for the same money — which the sweep has nobody to collect.

**Doc shapes.** `contract:{id}` (container; corrupt → fail boot). `meta` gains
`next_contract_id`. `EconomicContract` gains, all `#[serde(default)]`: `name`,
`created_at`, `amendments`, `disputed_at`, `roster_locked`,
`deposits: Vec<DepositRecord>`, `agreements: Vec<PartyAgreement>`,
`roles: ContractRoles`. `ConsentSurface` gains
`adjudicator: Option<AttestorSet>` and `evidence: Vec<EvidenceKind>`; `ContractTerms`
gains `outcomes: Vec<OutcomeTerms>`. No record type changed its **journalled** bytes,
so every E2/E3/E4a/E4b1 journal still replays.

**The visibility variant (doc 10 §15 / T6).** `EconomicVisibility` gains
`Audience { members: Vec<PrincipalId> }` and loses `Copy`; the three shipped variants
keep their serde bytes exactly (a unit variant still serializes as `"Parties"`), which
is what keeps both digest pins green and every stored plan hash resolving.
`canonical_visibility` sorts and deduplicates an audience by `principal_key` inside
`consent_hash`, so two orderings of one audience are one consent surface.
`may_view_contract` = staff · the issuer · a party · an audience member · **every
named attestor** (the adjudicator, and whoever each `Attestation` condition names) ·
a `contract.*`/`economy.account.view` holder on the contract's own compartment ·
anyone at all when the surface says `Public`. The attestor clause is the same shape
as the rest: naming somebody as an authority over a contract and then hiding it from
them makes the authority unusable. Party and audience
membership ask `WorldAuth::acts_for`, not identity, for the reason E4b1's fold gave
for holds (A2): an org party would otherwise be invisible to every person in it.
Everyone else gets `NamedResource::Contract`'s "No such contract." — identical for a
contract that does not exist and one they may not see.

**Deviations from §5, and why.**

1. **An `Offered` contract that lapses always refunds**, whatever `on_expiry` says.
   Forfeiting money escrowed toward an agreement that never bound anybody is a
   confiscation, not a forfeit; `Expiry::Forfeit` and `Expiry::Dispute` therefore
   apply only to a **fully accepted** contract.
2. **The builders are `Draft`-only by default**, exactly as §5's block says, and
   `@contract/offer` sets `roster_locked`. §3.5's "an `Offered` contract may carry an
   unfrozen party" is an explicit act — `@contract/roster <id>=open` — so "the surface
   changed after Offer is refused" stays the default it has to be. The **first
   acceptance** re-locks the roster (08e answers 5/7).
3. **An amendment voids every acceptance, not only the affected ones.** An acceptance
   binds to the whole surface's hash — that is E4a's money-safety spine — so
   `@contract/amend` refunds every escrow placed so far, clears the acceptances,
   re-opens the roster and returns the contract to `Draft`. 08c §3.5's "re-acceptance
   from the affected parties only" would need per-party binding, which is a change to
   the consent model rather than to this cut; recorded as an E6 question.
4. **A named adjudicator *replaces* each condition's own attestor set** rather than
   being an extra conjunct. Conjunction is unsatisfiable in the shape the answer
   describes (a runner's `Party` condition plus a third-party adjudicator), and
   "attestation from anyone else is refused" reads as replacement.
   `condition_holds` honours it symmetrically, so the adjudicator's word re-verifies.
5. **Outcome conditions are excluded from the all-conditions gate.** With an outcome
   table the conditions some outcome's `when` names decide *which* result happened and
   are mutually exclusive by construction; requiring all of them would make every
   outcome-indexed contract unsettleable. Every other condition stays a **gating**
   condition that must hold whatever the outcome. An empty table is E4a's behaviour
   exactly.
6. **`OutcomeTerms::expense_treatment` is declarative.** It is frozen inside the
   consent hash (so E6a cannot change it after acceptance) and nothing in E4b2 reads
   it: an outcome's `payouts` must already say where an advance ends up.
   `asset_consequences` **is** live — it names the deliverable indexes whose title
   moves that outcome causes, and a `Collateral` release is deliberately not
   outcome-gated, because a pledge comes back whichever way the contract went.
7. **`@contract/cancel` on a live contract is staff-only.** A contract in force is not
   one party's to tear up; the party routes are `@contract/dispute` plus unanimous
   `@contract/agree <id>=refund` (level 0), or the deadline.
8. **`@contract/attest` is accepted while `Disputed`.** Evidence usually arrives
   *because* somebody disputed, and recording it is what lets a resolution settle.
   It settles nothing: `promote_and_settle` only ever moves an `Active`/`ReadyToSettle`
   contract, so a disputed contract's escrow stays frozen.
9. **Syntax made concrete** where §5 was shorthand: `@contract/deliver <id>=title
   <obj> <from>-><to>` accepts `->` as well as `→`, and gains an `obligation <debit>
   <credit> <amount> every=<secs>` form (§5 needs the lease's template and named no
   syntax for it); a share is a bare number (exact) or `N‰`/`Npm` (permille);
   `@contract/condition <id>=attest <who>` takes `party:<principal>` |
   `cap:<capability>[:<scope>]` | `staff` | `canon:<kind>[:<source>]`. Four verbs are
   added beyond §5's list for the §3.5 extensions: `/roster`, `/amend`,
   `/adjudicator`, `/evidence`, `/outcome`, `/role`.
10. **Boot's phantom-`Settled` record is minted by boot**, like E4b1's obligation
    reconciliation records, not through `sweep::page` — the ladder's dedupe/reopen
    machinery needs a `Ctx` the boot path does not have, and boot's own
    `MAX_BOOT_ESCALATIONS` cap (now shared across both reconciliation passes) is the
    bound there. Every **runtime** contract page — a dispute past `DISPUTE_TTL`,
    settle attempts exhausted, a refused activation, a refused forfeit, a refused
    refund — goes through `sweep::page` and therefore through subject dedupe and the
    reopen-rather-than-re-mint rule.

**Verify-fold deltas (E4b2 verify fold).** The adversarial verify's confirmed
findings, folded in. The load-bearing one is the **terminal-transition rule**, which
replaces §5.1's "the state change comes first, then the money":

> A terminal transition is a **post under the contract's own deterministic key at its
> generation, followed by the state write.** While the post has not succeeded the
> contract stays live and carries `pending: Option<PendingTerminal { kind: Cancel |
> Expire | Forfeit, attempts, since }>`. The sweep retries a pending terminal every
> fire (step 3's `Retry`, which outranks every other step) up to
> `MAX_SETTLE_ATTEMPTS` and then `Disputed`; boot finishes a half-done one by looking
> the same key up in the idempotency index. **A contract is never terminal while its
> escrow is still reserved.**

The old order existed because the terminal state is what lifts the `DepositInto`
custody lock, so inverting it needs one new seam: `AuthEval::authorize_custody_unlock
(object, vault) -> bool`, `false` by default and for `WorldAuth`, answered `true` only
by `ContractAuth` with purpose `Unwind` for a deposit **that contract** recorded. It is
unforgeable from a command, because only this crate can inject a `ContractAuth`.

The rest, by area:

- **Escrow and the accept.** `@contract/accept` writes the contract doc **in the escrow
  post's own batch** (`api::post_with_companions`, whose companions ride
  `install_commit`'s write batch and its post-image snapshot). The hold ids are
  predicted from `peek_next_hold` — the very floor `validate` allocates from — and
  healed if they ever differ. Boot gains an **orphan-escrow pass**: a journal-`Held`
  hold with `purpose: Escrow { contract }` that the contract does not list is adopted
  when it qualifies and the contract is non-terminal; on an ended contract it is
  adopted, the key **generation advanced** (the old refund key is spent) and `pending`
  set, so the contract's own unwind hands it back; anything else pages.
- **Amounts, not membership.** `ContractPurpose::Settle` carries `(account, amount)`
  pairs netted per account (and `Forfeit` carries `(to, total)`), so a credit must be
  **exactly** what the consented split owes that account.
- **Consent for a pledge.** `authorize_asset`'s `Encumber` arm requires
  `asset.title_owner == from` and `contract.accepted_by(from)`; `Release` stays
  unconditional (a pledge always comes back). `check_offerable` proves every
  `Collateral { object, from }` has `from` a party holding title.
- **The pay rule.** `@contract/pay` names only an account the actor
  `may_view_account`s; a **listed party** may add their own consideration to a `Draft`
  (self-consideration), which is how a payer whose wallet the issuer cannot see gets
  into the surface. Every `consideration.from` must be a party
  (`check_structure`). `/show` derives a payer's name from the **party list**, never
  from an account's owner.
- **Feasibility, split.** `check_offerable` splits into `check_structure` (pure: caps,
  split and outcome arithmetic, parties, payers, the condition graph and its cycles,
  resolvable clocks, satisfiable attestor sets) plus the world half. Boot re-runs the
  pure half on every non-terminal record and quarantines what it refuses; `save_open`
  re-runs the whole of `check_offerable` on an `Offered` contract (so `/roster open`
  cannot re-sign an infeasible surface) and re-notifies the parties.
- **Authority.** `authorize_attestor`'s `Party(p)` arm uses `acts_for` when `p` is an
  **organization** (identity still, for a person); `contract_attest` records the
  **party**, not the officer, so the attestation still stands once given, and refuses
  a second attestation of a filled slot unless the recorded attestor no longer
  qualifies (an `After` anchor cannot be pushed forward). `acting_party` gains the same
  organization-only `acts_for` fallback, so an org party on an **unscoped** contract can
  act. `/agree` requires `contract.accept`, not `contract.dispute`.
- **Disclosure.** Every contract refusal routes through `commands::refuse_named`, so an
  unviewable and a nonexistent vault/account/asset render identically at draft and at
  offer. `contract_show` renders **per class**: party/issuer/attestor/staff get the
  full surface with every account id masked by `may_view_account`; an **audience** gets
  a *notice* (name, state, window, condition kinds — no money, roles or hold ids; the
  issuer only when the new `EconomicVisibility::Audience { announce_issuer }`, inside
  the consent surface, says so); a public stranger gets the consented surface with ids
  masked. A settlement refusal shown to a party collapses to "staff have been paged"
  when it names something they may not view.
- **Deposits.** The returns are derived at unwind time from the journal's
  `last_custody`, so a deposit delivered **after** activation still goes home;
  `record_deposits` is only a cache now.
- **The sweep.** Step 3 filters records with nothing due **before** the per-fire cap (a
  `Disputed` past its TTL counts only when its escalation is `renotify_due`/
  `reopen_due`) and rotates the scan with a durable `meta.sweep_contract_cursor`, so id
  order cannot starve a high id.
- **Boot.** `Settled` additionally requires the named transaction's idempotency key to
  be **this contract's** `settle_key` at its generation; a doc claiming another
  contract's settlement freezes as drift. `dispute_with` pages from every state,
  terminal included.

**§5.x — owner abandonment (doc 10 T3) is deferred to E7, and precisely why.**

The brief's condition was *"if E3's asset record admits an untitled state cheaply"*.
It does not. `EconomicAsset::title_owner` is a non-optional `PrincipalId` with **53
call sites across 14 files**, five of them test files and one of them outside the
crate (`mush-sim/tests/custody.rs`); it keys the derived `assets_by_owner` index, it
is compared field-by-field by boot's `journal_fields_eq` drift detector, and it is
load-bearing in invariants 12 (encumbered transfer), 13 (assignment), 18 (scope
coherence), 19–20 (authorization), and 22 (acquisition provenance), plus the boot
liveness pass, the destruction backstop, `WorldCustody::may_deposit`, and
`mush-faction`'s read-through. Turning it into `Option<PrincipalId>` is a
money-adjacent refactor of E3's verified surface, not a variant addition.

The two cheap encodings were both rejected as *unfaithful*, which is worse than
absent: a new `AssetLifecycle::Abandoned` leaves `title_owner` naming a former owner
(so "title → none" is a lie the record does not tell) **and** collides with E3's rule
that a non-`Active` asset takes no asset op, which would make the salvage claim T3
describes — an ordinary contract that turns possession into title — impossible to
write; and transferring title to a registered `sys:` principal is an owner, not the
absence of one, and would need staff authority on every abandonment.

What E7 needs, recorded so it is a mechanical job rather than a rediscovery:
`title_owner: Option<PrincipalId>`, `assets_by_owner` keyed only for `Some`,
`journal_fields_eq` unchanged (it compares the field either way), the five invariants
above reading `Some(owner)`, `AssetEffectKind::Abandoned` + `AssetOp::Abandon
{ object }` (owner-only, refused while encumbered, clearing assignment and
maintenance, journaled with the full post-state like every other effect), and
`AssetOp::Title`/`TransferTitle` accepting an untitled-but-live asset so a salvage
contract can re-title it.

**Handoffs.**

- **E4c** takes three seams this cut left open at their fail-closed defaults. (a) The
  `FactProvider`: `TypedCondition::Fact` is accepted at draft and resolves `None`
  through `NoFacts`, which is unsatisfied, so a `Fact`-gated contract cannot settle
  until E4c wires a provider. (b) The canon observer: `AttestorRef::Canon` is matched
  on kind **and** source by `condition_holds`, and `WorldAuth::authorize_attestor`
  answers `false` for `AttestorSet::Canon` deliberately — a canon attestation is
  recorded by the observer from a canon event, never claimed by a principal. (c) The
  notification seam: `contract_live::notify` reaches only `PrincipalId::Object`, so an
  organization or system party is told nothing today, exactly as `sweep::notify_principal`.
  E4c's canon bridge should read `EconomicVisibility::is_public()` — an
  `Audience` list is emphatically **not** public and must never reach canon.
- **E6a/E6b** get the record shapes they asked for and two open questions:
  per-party acceptance binding (deviation 3) and a real consumer for
  `ExpenseTreatment` (deviation 6). The party subsystem (08e answer 21) stays a
  separate crate; E4b2's roster lock is the economy-side half of it, and a run
  contract consumes a frozen roster by being amended, not by pointing at a party
  record.
- **E5** is unaffected except that a `Contract`-caused obligation is now a real thing
  boot can see: E4b1's rule "a `Contract` cause whose contract is not live →
  `Suspended`" was vacuously true (no contracts loaded) and is now the live check,
  and `@contract/cancel`/expiry cancels a lease's rent outright.

---

### 5.2 E4b2 verify outcome

An adversarial verify ran three lenses over the cut — **A** command authority and
existence hiding, **M** consent and settlement money, **B** durability, boot and the
sweep's contract step. Twenty-two findings were confirmed by probe and all are folded
in above, with one regression test each in `tests/e4b2.rs` (`vf_<lens><n>_...`, 22
tests; the file is **59**, `mush-economy` **368**, both digest pins green).

**Lens A (8).** A1 `@contract/pay` was an account existence-and-owner oracle (HIGH).
A2 `contract_refusal` bypassed the hiding renderer, so `/offer` probed vaults and
wallets. A3 an org attestor could never attest, and an attestor set nobody could
satisfy was offerable. A4 an org party on an unscoped contract could never accept,
dispute or agree. A5 `/agree` took the *freeze* right rather than the disposition
right. A6 a settlement refusal reached a party raw. A7 an `Audience` — doc 10 T6's
eligible defenders — was rendered exactly like a party, plan and all. A8 the help text
and the parser disagreed about `condition … title`.

**Lens M (8).** M1 a contract could encumber a bystander's asset (HIGH). M2+M3 a
refused forfeit or unwind left a **terminal** contract holding live escrow that no
command could reach, and the failure was logged rather than paged (HIGH). M4 credits
were membership-checked, so a hand-built plan could route the whole escrow to
`remainder_to`. M5 an attestation could be overwritten, which moved an `After` anchor
and could defer a consented settlement indefinitely. M6 deposit return worked only for
pre-activation deposits. M7 `/roster open` let the builders re-sign a surface with no
re-validation. M8 a consideration from a non-party was offerable and unfundable.

**Lens B (6).** B1 boot disputed a correctly settled or refunded contract whose doc
write was lost (HIGH). B2 a lost `accept` doc write orphaned the escrow forever (HIGH).
B3 a refused forfeit was silent and terminal (HIGH; closed by M2+M3's design). B4 step
3 starved a high-id contract behind a wall of disputes. B5 the `Settled` check was
existence-only. B6 boot trusted a stored surface.

**Handoffs.**

- **E4c**'s disclosure filter renders through the **same per-class rule** as
  `contract_show` (`ShowClass::{Full, Notice, Consented}` plus the `may_view_account`
  mask), rather than inventing a second, weaker one; its canon bridge still reads
  `is_public()`, and an `Audience` is emphatically not public.
- **E6a** inherits the **audience notice render** for run boards — a board is a notice
  surface, not a terms surface — and the still-open **per-party binding** question for
  amendments (deviation 3): today an amendment voids and re-collects *every*
  acceptance, which is right for a changed split and heavy for a changed roster.
- **E7** inherits owner abandonment (§5.x), unchanged by this fold.

---

## 6. E4c — facts, events, evidence, the disclosure filter

1. **`FactProvider` seam** (economy-defined, E1c-shaped: `provider() -> &str`,
   `resolve(&self, w, kind, id) -> Option<FactState { satisfied: bool, actor: Option<ObjId>, at: i64 }>`,
   frozen via `EconomyModule::with_facts(..)`). `mush-jobs` implements `JobsFactProvider` for
   `kind = "resolved"` **only for payload-bearing jobs** (a payload-less job can be reopened,
   `mush-jobs/src/commands.rs:583-593` — not a terminal fact).
2. **Canon attestation observer**: economy subscribes to `WorldEventKind::CanonRecorded` (core's
   event, `mush-core/src/module.rs:388-422`, `Registrar::subscribe` at `:1431-1437`; the only
   production emitter is `mush-canon/src/lib.rs:159`, which persists then emits within the same
   command step and is documented as terminal; **E4c adds `source: String` to the event** — §2) and,
   in the same step — hence the same `WriteBatch`
   as canon's own writes — records an `Attestation { source: canon:{id} }` on every `Active`
   contract condition `Attestation { by: Canon { kind, source } }` whose `kind` **and `source`**
   match the recorded canon event (E5's owner answer 20: progress must cite a specific front,
   opportunity, or operation — a `scene.concluded` involving any faction member is too broad, so
   kind + entities is **not** a match key), then attempts settlement. **The observer is infallible from dispatch's point of view**: any internal refusal
   is logged and swallowed; the attestation write stands (durable) and the contract stays
   `ReadyToSettle` for the sweep — canon's terminal-commit contract is never violated by an
   economy error. Canon's RAM-tail-only queryability does not matter: the attestation is durable
   on the contract doc.
3. **`EconomyObserver` seam + compact events** (§12.4): `EconomicEvent::{TransactionPosted,
   HoldChanged, ContractChanged, ObligationChanged, AssetTitleChanged}` with **ids and kinds only** —
   no amounts, no parties, no `cause` text (D-E4-13) — delivered post-commit to observers frozen at
   wiring. `mush-server` implements the **canon bridge** (D-E4-12): for a settlement or obligation
   transition of a contract whose consented `visibility` is `Public` (`EconomicVisibility` is the
   shipped closed enum, `record.rs:289-300`), commit a `CanonDraft { kind: "economy.settled" |
   "economy.delinquent", entities: parties, scope: existence-level, detail: existence-level only }`.
   Every party accepted that disclosure with the consent surface; `Parties` (the default) and
   `Staff` contracts never reach canon.
4. **Approvals → org audit** (E3 leftover): a new five-line `mush_org::audit(ctx, OrgAction)`
   wrapper over the tail of `submit` (`mush-org/src/api.rs:185-193`; no such fn exists today);
   `@econ/approve|deny` append an entry naming request id and account scope.
5. **Disclosure filter, first cut**: `@econ/history <account> [n]` for non-staff — the journal
   entries touching an account the actor may view, rendered per `EconomicVisibility`: the viewer's
   own side in full; the counterparty named only when the viewer may view that account too, else
   "another account"; `Staff`-visibility entries omitted entirely (not "hidden"); descriptions
   shown only to parties. `EconomicVisibility` is thereby **enforced for reads**; recording is
   unchanged. `@contract/show` and `@hold/show` render through the same rule.
   **The attestation seam is what E6b gates on** (08e §3 answers 6, 19): a run may settle from a
   sealed scene outcome (a typed outcome record `mush-scenes` will commit as canon with
   `source: scene:{uid}` — produced by E6b, or by E4c if the seam needs a real producer to test
   against), an authorized attestation, a deterministic condition, or a composite; E4c must accept
   all four through the same `TypedCondition` alphabet and refuse prose-only settlement.
6. **Title issuance for registered system principals** (fed back by E5, 08d §7 item 4; D-E3-13 was
   "shape only" until now): `mush_economy::issue_title(ctx, object, owner, scope, issuer:
   SystemPrincipalId)` mints an `EconomicAsset` title record as a journaled `Created` effect with
   cause `Internal { kind: "title.issue", id }`, authorized only for a system principal registered
   as a title issuer at boot (`EconomyModule::with_title_issuers(..)` — the frozen-table pattern);
   a ruleset (E5's `system:wod.housing`, E7's production output) registers its issuer. `@asset/title`
   stays the staff path. Tests: an unregistered issuer refused; the effect carries the issuer.

Tests (`tests/e4c.rs` + one in `mush-jobs`, ~15 — built: **24**, in three files, see §6.1): a payload
job's approval satisfies a `Fact` condition and a payload-less job's does not; a canon commit
attests and settles in one batch (`drain_write_batch`) and an internal refusal inside the observer
leaves canon's write intact; events carry no amounts; the canon bridge emits only for `Public` and
only existence-level detail; `@econ/history` hides counterparties the viewer may not view and omits
staff-visibility entries; approve/deny land in the org audit ledger.

### 6.1 As built (E4c)

The cut connects the economy to the rest of the world. Six seams, **two** core changes
(the second folded in after the verify — see below), and the read side of
`EconomicVisibility` — which had been *recorded* since E2 and is enforced from here on.
**49** tests across three files after the verify fold (`mush-economy/tests/e4c.rs`
**35**, `mush-server/tests/e4c_bridge.rs` **11**, `mush-jobs/tests/e4c_facts.rs` **3**);
`mush-economy` is **403**, and both digest pins (E2 `9c917eee…`, E3-shaped
`b780e3a0…`) are green.

**The first `mush-core` change, in full.** `WorldEvent::CanonRecorded` gains
`source: String` (`mush-core/src/module.rs`), `mush-canon`'s emitter passes
`draft.source` through (`mush-canon/src/lib.rs`), and the one constructing test site
(`mush-faction/tests/simulation.rs`) names the new field. Three files, four lines of
code. `WorldEvent` derives no serde at all — it is a synchronous in-process
notification, never persisted — so there is no back-compat surface to keep; the two
consumers that destructure it (`mush_faction::on_canon_recorded`, and the economy's
new one) both use `..`. Nothing else in `mush-core` was touched, and one thing that
wanted to be is recorded instead rather than built (below).

**New files and signatures.**

- `src/facts.rs` — `FactState { satisfied, actor, at }`, the `FactProvider` trait
  (`provider() -> &str`, `resolve(&self, w, kind, id, revision) -> Option<FactState>`),
  the `FactTable`, and `WorldFacts`, which implements E4a's `FactEval` over it.
  Shaped for doc 10 §15 rather than for jobs: `kind` is a **namespaced string** so
  `sim.journey.arrived` and `tactics.unit.destroyed` need no economy change,
  `id` is the provider's own stable id (the economy's reference points *at* the
  mechanical journal and never duplicates it), and `revision` is passed through so a
  provider may refuse a stale reference. Wired at every `ValidateEnv` site
  (`api::post_with_companions`, `contract_live`'s two, `sweep::step_contracts`); the
  reversal path keeps `NoFacts`, which is correct — a reversal re-validates an
  existing transaction and has no conditions. An empty table answers `None` for
  everything, which is exactly the `NoFacts` behaviour it replaced.
- `src/observer.rs` — the `EconomyObserver` trait, the `ObserverTable`, `emit`, and
  `MAX_OBSERVER_DEPTH` (4).
- `src/attest.rs` — the `CanonRecorded` observer.
- `contract.rs` gains **`canon_set_accepts(set, kind, source)`**, the single predicate
  the observer and `attestation_matches` share, so what "the right canon record" means
  cannot drift between recording an attestation and re-verifying it.
- `api.rs` gains `issue_title(ctx, object, owner, scope, issuer)` and the private
  `TitleIssuerAuth`; `contract_live.rs` gains `promote_and_settle_with` / `settle_with`
  (companion docs riding the settlement post's batch).
- `mush_org::audit(ctx, OrgAction)` — the tail of `submit` without a plan.
- `mush_jobs::JobsFactProvider`; `mush_server::econ_canon::CanonEvidenceBridge`.
- `EconomyStore` gains `facts`, `observers`, `title_issuers` and the RAM-only
  `event_depth`; `EconomyModule` gains `with_facts`, `with_observers` and
  `with_title_issuers`. **Module order in `modules.rs` is unchanged** — the three new
  tables hang off the existing `EconomyModule` construction.

**The compact events.** `EconomicEvent::ContractChanged` gains
`transaction: Option<TransactionId>` (`#[serde(default)]`); no other variant changed.
Emission points are deliberately few: `api::finish` announces
`TransactionPosted` + one `HoldChanged` per hold effect + one `AssetTitleChanged` per
asset effect **after `install_commit` and after the strict durable gate**, and
`put_contract` / `put_obligation` announce a **state transition** — a *change of
kind*, not a write. That distinction is load-bearing: a contract document is rewritten
by every builder verb, every acceptance and every failed settle attempt, and an
obligation by every settled period, so publishing on write would make the canon bridge
a firehose. Events carry ids and kinds only (D-E4-13) and are read back out of the
**indexed transaction**, never the plan, so an event can only describe something the
journal has.

**The bridge's re-entry rule** (`mush-server/src/econ_canon.rs`). The bridge commits
canon and does **nothing else** — it never posts, never settles, never writes an
economy document. `mush_canon::commit` is documented as terminal: it appends, then
emits `CanonRecorded` synchronously, and the economy's own attestation observer is
subscribed to that. A bridge that also called back into the economy would be
re-entering, from inside an economy event callback, the machinery that produced the
event. The chain that *does* exist — settlement → event → canon → `CanonRecorded` → a
**different** contract's attestation → its settlement — is finite because each step
consumes a distinct contract and carries a new transaction id, and it is bounded
anyway by `MAX_OBSERVER_DEPTH`. Work the bridge might want from the economy belongs in
the **next step** (a command, or the sweep), because by the time an observer runs the
post is installed and its batch is closing: nothing added there can be part of the
decision it records. Publication is keyed on the **landed transaction id**
(`source: economy.tx:{id}`) and refuses to commit a draft whose source is already in
canon's tail, so the sweep's `AlreadyPosted` retry and boot's `finished_transition`
publish nothing new.

**Deviations from §6, and why.**

1. **The tests are three files, not one.** `mush-economy` must not depend on
   `mush-canon` — the dependency direction is the whole reason the observer subscribes
   to *core's* event — so a test that commits real canon cannot live in the economy
   crate, not even as a dev-dependency (the same rule the grounding review applied to
   `mush-jobs`). `tests/e4c.rs` drives `CanonRecorded` exactly as core delivers it,
   with a canon-namespace write queued on the same step to stand in for canon's append;
   `mush-server/tests/e4c_bridge.rs` runs the real three-crate loop.
2. **The observer accepts `Disputed` as well as `Active`/`ReadyToSettle`**, matching
   `@contract/attest` (§5.1 deviation 8) rather than §6.2's "every `Active`". Evidence
   usually arrives *because* somebody disputed, and recording it is what lets a
   resolution settle; it settles nothing, because `promote_and_settle` only ever moves
   an `Active`/`ReadyToSettle` contract.
3. **A named adjudicator replaces the condition's own canon set here too.** §5.1's
   deviation 4 applied the rule to people; applying it asymmetrically would let a canon
   record discharge a condition the parties had consented to put in one adjudicator's
   hands.
4. **`@econ/history` counts an effect-only entry as "touching" the account.** An escrow
   placement moves `held_amount` and no journal *line* at all, so a line-only filter
   made the one entry a payer most wants to see invisible — and made the
   staff-visibility rule untestable, since the escrow post is the only entry a
   `Staff`-visibility contract produces.
5. **A masked counterparty carries no amount.** §6.5 says the counterparty is named
   only to a viewer who may view it; the render also drops the *amount* of a line it
   masks, because a five-way split would otherwise tell each runner what every other
   runner was paid. In a two-line transfer the mirror amount is arithmetic rather than
   disclosure; the id is what is withheld either way.
6. **"Party" is narrower than "may view".** A description is shown to someone the
   entry's money is *about* — the poster, or anyone who `acts_for` an owner of an
   account it moved — not to everyone who may read the account. An auditor holding
   `economy.account.view` reads what moved and not the free text somebody wrote about
   why. An `Audience` widens that set to the named audience and is treated exactly like
   `Parties` for anyone outside it.
7. **`issue_title` refuses at the API, and again at the engine seam.** The registered
   table is checked before anything is built, and `TitleIssuerAuth` — which authorizes
   exactly one `AssetOp::Title`, for exactly the object/owner/scope named, on behalf of
   exactly that issuer, and keeps the trait's `false` default for every other method —
   is what `validate` asks. It grants no money authority at all. The world half of
   invariant 18 (scope coherence) is checked in `issue_title` itself, because the
   injected authority holds no `&World`.
8. **The bridge's replay guard reads canon's RAM tail.** Canon has no id→event query
   and its deep history pages through a store-read seam that does not exist yet; canon
   events are sparse enough that a settlement's own line stays in the tail for a very
   long time. A tail that has rolled past it is the documented limit — the same limit
   canon's own id-allocator safety net carries.

**Recorded, not built (the "one core change" rule).** The `@econ/history` renderer
would read better with a shared "how long ago" helper; `mush_jobs::fmt_ago` is exactly
it and lives in the wrong crate for the economy to use. Moving it to `mush-core` is a
second core change for a cosmetic gain, so the render prints the raw `posted_at`
stamp and the move is E5's if anything else wants it.

**Verify-fold deltas (2026-09-03).** The end-of-build adversarial verify ran three
lenses over the cut; §6.2 records the outcome, and this is what the as-built above
now says instead.

- **A canon attestor must be a producer-declared outcome kind.**
  `CanonOutcomeKinds` is a fourth wiring-frozen table
  (`EconomyModule::with_canon_outcome_kinds`), checked by `check_structure` at
  `@contract/offer` and at boot. `scene.concluded` is a *lifecycle* record any
  participant can produce, so it is not on it — the server registers **nothing**
  until E6b ships `scene.outcome` sealed under facilitator authority. The observer
  additionally requires the record's `entities` to intersect (or act for) the
  contract's own parties. §6's E6b handoff is rewritten accordingly: `scene.concluded`
  never attests.
- **`Canon { source }` is mandatory.** `parse_attestor` refuses `canon:<kind>` with no
  source, `check_structure` refuses a stored one, and `/adjudicator` goes through the
  same parser. A set naming no source was a wildcard over every record of its kind.
- **`jobs.resolved` answers from an applied-payload marker.** `Job.payload_applied` is
  stamped by `+job/approve` in the same batch as the handler's own mutations;
  `+job/status` refuses `=resolved` on a payload job and refuses any move off
  `Resolved`. A status is a state; the marker is a fact about the past.
- **Canon evidence is re-scannable.** `EconomyStore.canon_tail` (RAM-only, 64 records)
  remembers every `CanonRecorded` the world announced, including while frozen;
  `@econ/unfreeze` and a contract's **activation** re-scan it, and
  `@contract/resolve <id>=settle [reason]` records an `AttestorRef::StaffOverride` on
  every unmet `Attestation` condition (canon sets included) as the level-2 answer past
  the tail. Deterministic conditions are still re-verified and still refuse.
- **Deterministic conditions carry an observed-at stamp.** `stamp_observed` fills the
  satisfaction slot of a `Fact`/`DepositInto`/`TitleHeld`/`Before`/`After` the first
  sweep that finds it holding, with `AttestorRef::Observed { clock }` on the clock the
  anchoring `After` will compare against — so "pay 60 s after the job is approved"
  matures. The condition is still re-evaluated live at settlement.
- **`issue_title` checks the world and the generation.** The object must be a live,
  non-`GOING` world object (the staff path's rule), and the idempotency key is
  `title.issue/{object}/{n}` where `n` counts prior `Created` title effects in the
  journal; the engine lets a **written-off** title be re-issued (with no live
  encumbrances), so issue → write-off → re-issue mints the next generation instead of
  reporting `AlreadyPosted` and minting nothing.
- **`with_facts` refuses duplicate provider namespaces** (a wiring panic —
  composition-root data), and **`MAX_OBSERVER_SETTLEMENTS_PER_EVENT`** (8) bounds how
  many contracts one canon record settles inside the observer; past it the attestation
  still stands and the sweep finishes the job.
- **`STALE_CONTRACT_AGE_SECS`** (7 d, §9): a live contract with no condition progress
  for that long is paged at level 2, deduped on the subject, recommending
  `@contract/dispute` then `@contract/resolve`.
- **The observer depth guard is RAII and loud.** The counter is thread-local behind a
  drop guard (a panicking observer used to leave it raised forever, silently dropping
  every event afterwards), and hitting `MAX_OBSERVER_DEPTH` now pages level 2 with the
  contract whose emission was dropped as the subject.
- **`Outbox::pin_deferred` — the cut's second and last `mush-core` change.**
  `mush_canon::commit` pins the deferred prefix at its end, and `run_handler`'s abort
  path truncates only past the pin. Canon is terminal by contract, so nothing that rode
  with it may be dropped when the command later returns `Err`; RAM is not rolled back,
  and dropping those writes left the store behind a world that had already moved. Two
  files in `mush-core` (`output.rs` — a field, `pin_deferred`, `deferred_pin`, and a
  `max` in `truncate_deferred`; `cmd.rs` — a comment), one line in `mush-canon`.
- **Publication is no longer best-effort.** `EconomicContract.canon_published:
  Option<SmolStr>` (`#[serde(default)]`) is written by `mush_economy::mark_canon_published`
  — the bridge's one call back into the economy, and the only thing it may write — after
  a `mush_canon::commit` that **succeeded**. The bridge de-dupes on that marker rather
  than canon's rolling RAM tail; a failed commit logs `tracing::error!` and leaves it
  unset, and the sweep re-emits `ContractChanged { Settled }` for `Public` contracts
  whose marker is unset and whose landing is at least one sweep old
  (`MAX_REPUBLISH_PER_SWEEP` = 8). That is also what finally chronicles a settlement
  boot's `finished_transition` healed, which emits no event at all. The delinquency
  draft is keyed `economy.obligation:{id}:{missed_total}`, so a second delinquency on
  one lease is a second line.
- **The reservation-visible / contract-hidden rule** (D3): *a reservation on an account
  is a fact its viewers may see; the contract behind it is not.* For an
  `Escrow { contract }` hold whose contract is `Staff`-visibility (or whose viewer is
  outside its disclosure class), a non-staff `@hold/show` prints the amount and state
  with no purpose line and no beneficiary, and `@econ/history` relaxes rule 4 for an
  **effect-only** entry (the placement is shown; its description is not).
  `@obligation/show|list` mask both account ids through `mask_account`;
  `@contract/deliver … obligation` resolves both accounts through the `hidden_text`
  loop first (an account a **party** owns counts as known, or a lease's rent would be
  undraftable by anybody but staff); `@econ/history`'s `n` is validated to `1..=20`
  rather than clamped; a viewer admitted to a non-`Public` contract only through
  `economy.account.view` on its scope gets `Notice`, and a `Notice` viewer's `/list`
  drops the party count while `render_state` drops the settlement transaction id.
- **Masked-line collapse** (D4). Every masked counterparty on one `@econ/history` entry
  collapses into a single `other accounts` token — the finding asked for consecutive
  runs; collapsing them all is strictly stronger, because a run whose masked lines
  straddle the viewer's own would otherwise still publish "there are others on both
  sides". The accepted residual limit is the **two-line** case, where the mirror amount
  is arithmetic rather than disclosure.
- **Doc-only (E11/E12).** Boot's `finished_transition` restores *state*, not evidence:
  it reads a deterministic key out of the idempotency index and moves the contract to
  the transition that already posted, and it never fabricates an attestation, so it has
  no money consequence beyond the one the journal already carries. And `FactProvider`'s
  contract is stated on the trait: **never panic, never block, composition-root
  trusted** — it is consulted inside validation, on the world task, with the store
  borrowed.

**Handoffs.**

- **E5** takes the fact seam's first real registration beyond jobs and the title
  issuer's first real entry: `EconomyModule::with_title_issuers(vec![
  SystemPrincipalId::new("wod.housing")])` is the shape, and the server currently
  passes an empty list on purpose (`@asset/title` stays the staff path until a ruleset
  owns one). `@econ/cadence` and the `Delinquent` consequence hook are unchanged from
  §4.7's handoff.
- **E6b** gates on the attestation seam and now has it — and, after the verify fold,
  **owes it a producer.** All four of 08e answer 6's routes go through the one
  `TypedCondition` alphabet: a **sealed scene outcome** is a typed canon record with
  `source: scene:{uid}` matched by an `Attestation { by: Canon { kind, source } }`
  condition; an **authorized attestation** is `@contract/attest` under a
  `Party`/`Capability`/`Staff` set; a **deterministic condition** is
  `Fact`/`TitleHeld`/`DepositInto`/`Before`/`After`; a **composite** is several of them
  on one contract. Prose settles nothing: a canon record with no matching typed
  condition attests nothing at all, whatever it says.
  **`scene.concluded` never attests** (fold E1). It is a *lifecycle* record — any
  participant can conclude a scene — so a payee gated on one could seal their own
  settlement. E6b must build **`scene.outcome`**: outcome-bearing, sealed under
  facilitator/adjudicator authority, naming its subject at offer, and registered
  through `EconomyModule::with_canon_outcome_kinds`. The server registers **no** canon
  outcome kind until it ships, so every `canon:` attestor set is refused at
  `@contract/offer` in the meantime — fail-closed, and visible to whoever drafts it.
- **E7 / `mush-sim` / `mush-tactics`** register the doc 10 §15 boundary facts as
  `FactProvider`s. The kinds the plan names, so they are a registration rather than a
  rediscovery: `sim.journey.arrived`, `sim.sortie.flown`, `sim.station.served`
  (R6/T13's participation fact, which outcome-indexed terms already index on),
  `sim.delivery.completed`, `sim.docking.completed`, `tactics.unit.destroyed`,
  `tactics.salvage.claimed`. Each needs a stable id space in its own domain journal
  and should answer `None` — never `Some(false)` — for a record it cannot vouch for.
- **The notification seam is still open.** `contract_live::notify` and
  `sweep::notify_principal` still reach only `PrincipalId::Object`; the
  `EconomyObserver` seam is now the place an org or system party's notification, and
  the staff-channel escalation fan-out (§4.7 deviation 5), can be built without the
  economy learning about channels.

---

### 6.2 E4c verify outcome

Three lenses ran at end of build (2026-09-03) — the **disclosure filter** (D1–D8),
**settlement by evidence** (E1–E12), and **events / the canon bridge / durability of
emission** (V1–V6). Every confirmed finding is folded above with a regression test
named `vf_<lens><n>_…` in a `// ---- E4c verify fold ----` section
(`mush-economy/tests/e4c.rs` 17, `mush-server/tests/e4c_bridge.rs` 7,
`mush-jobs/tests/e4c_facts.rs` 1).

**Disclosure.** The cut enforced `EconomicVisibility` on the pages it was written for
and left three side doors: `@contract/deliver … obligation` answered three
distinguishable messages for an id (an account existence *and* faucet oracle, D1);
`@obligation/show|list` printed both account ids raw to anybody who could view either
one (D2); and `@hold/show` handed a `Staff`-visibility contract's id and beneficiary
to exactly the people `@contract/show` had just been taught to omit it from (D3). The
smaller three were arity (one `another account` per hidden line counted the split, D4),
class (an `economy.account.view` auditor read the full terms, D5; a notice viewer read
the party count and the settlement transaction id, D6), and shape (`n` clamped, so `0`
and `banana` were distinguishable, D8). The bridge named every party of a lease's
contract on a delinquency line (D7); it names the debtor now.

**Evidence.** The HIGH findings were all one shape — *something that is not an outcome
was being read as one.* `scene.concluded` is a lifecycle record a payee can produce
alone (E1); `Canon { source: None }` accepted every record of its kind (E2); and
`jobs.resolved` read a **status** staff can set by hand rather than the fact that a
payload ran (E3). Below them, evidence that arrived at the wrong moment was
unrecoverable (E4), an `After` anchored on a deterministic condition never matured
because a deterministic condition never filled its slot (E5), `issue_title` minted
title for any `ObjId` (E6) and silently minted nothing after a write-off (E7), a
duplicate provider namespace shadowed in silence (E8), a contract could wait forever
without anybody being told (E9), and one canon record could settle an unbounded number
of contracts inside a synchronous observer (E10).

**Events and durability.** A command that returned `Err` after committing canon dropped
canon's own append, the settlement's journal chunk and the contract document while RAM
kept all three (V1) — the cut's second `mush-core` change, `Outbox::pin_deferred`.
Publication was best-effort in three ways that share one cure (V2 + V3 + V5): a failed
commit was lost behind a `let _ =`, a boot-healed settlement emitted no event at all,
and the replay guard read a RAM tail that rolls. A second delinquency on one lease read
its own first line and said nothing (V4). And the observer's depth counter was not
panic-safe, so four panics silenced every economic event in the process, with the bound
itself logged and never paged (V6).

**Handoffs out of the fold.**

- **E6b** builds `scene.outcome` — a typed, outcome-bearing canon record sealed under
  **facilitator/adjudicator authority**, named at offer — and registers it through
  `EconomyModule::with_canon_outcome_kinds`. `scene.concluded` never attests; a run
  gated on one is refused at `@contract/offer` today.
- **E7 / `mush-sim` / `mush-tactics`** must answer their `FactProvider`s from **applied
  markers**, never from status: `jobs.resolved` reads `payload_applied`, and a sortie,
  journey or salvage fact has to name the moment the domain journal recorded the effect,
  not the state a record happens to be in now. A provider that can only report status
  should answer `None`.

## 7. Build discipline

Per cut: (1) confirm the cut's slice of this plan against the tree; (2) build, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo test --workspace`; (3) **a probe-confirming
adversarial verify of that cut** (three lenses, throwaway probe tests) before the next cut builds
on it; (4) fold with regression tests; (5) commit with the verify outcome; nothing pushed until the
owner says so. `cargo fmt` stays off until the pending reformat lands.

## 8. Decisions (review-confirmed unless marked)

- **D-E4-1 Holds are journal-authority effects; `held_amount` is a re-derived projection with its
  own seq anchor (`last_hold_seq`).** Same argument as D-E3-1; a hold has no recovery latch, so a
  missing doc heals silently. Doc 08 §8.1 sketches no journal/doc split for holds — this is where
  one is chosen.
- **D-E4-2 (revised, Draft 0.4) Four clocks: `RealUtc`, `WorldIc`, `Manual`, and economy-owned
  `Cadence(CadenceId)` records; `Registered { provider }` stays deferred.** E5's owner answer 7 asks
  for "a named campaign/domain clock with a configurable cadence and bounded catch-up" — that is a
  cadence clock, buildable inside the economy without a provider seam (`cadence:{id}` container
  docs, advanced by the sweep; the sweep stamps make catch-up classification work for it as for
  obligations). A provider-backed clock waits for E7's consumer. IC time enters the engine as
  `ValidateEnv.ic_now`; a cadence's position enters as its period index.
- **D-E4-3 One bounded sweep tick; catch-up from schedule arithmetic, never from tick count;
  per-period idempotency keys; excess periods are forgiven with a classified cause, not parked; the
  due pool excludes `Delinquent`/`Suspended`.** The core scheduler is in-RAM, re-armed at boot,
  fires each tick exactly once regardless of downtime, and cannot host per-record timers.
  `maximum_catch_up` is clamped at creation. *(Draft 0.2's `Reconciliation` wait state is gone —
  see D-E4-14.)*
- **D-E4-4 Contracts are the consent carrier; acceptance bound to the whole consent surface's hash
  is the settlement authority, re-checked live; conditions settle on attestations and the
  economy's own journaled facts, re-verified at settlement; `DepositInto` objects are
  custody-locked while the contract is live.** No stored authority; no synchronous money on
  foreign events; the canon observer records a durable attestation in canon's own batch.
- **D-E4-5 Obligation and contract *state* are doc-authority containers; their money is the
  journal.** Boot quarantines (`Suspended`/`Disputed`) rather than guessing; a `Settled` contract
  with no journal transaction freezes the economy.
- **D-E4-6 The sweep never claims durability.** Routine settlement is silent; only transitions
  notify, and a transition notice is not a money confirmation, so invariant 9 is not at stake and
  no per-actor durable gate (and no core change) is needed. *(Replaces Draft 0.1's core seam.)*
- **D-E4-7 The disclosure filter starts as a read-side renderer over the journal**, enforcing
  `EconomicVisibility` on `@econ/history` and contract/hold views; the evidence bridge lives in the
  server crate so economy depends on neither canon nor jobs.
- **D-E4-8 `Encumber` stays staff-only for manual liens; `Lease`/`Collateral` are contract-placed.**
- **D-E4-9 Available-balance enforcement replaces posted-balance enforcement for every debit**
  (invariant 23); a hold can never exceed available at placement and cannot be placed on an
  unbounded-credit account, where `permits` is vacuous.
- **D-E4-10 `ContractState::Accepted` is folded into `Active`** — the last acceptance activates.
- **D-E4-11 A hold placement is a debit-authority plan and is subject to the account's
  `ApprovalRule`**; an accept that needs approval records no acceptance and leaves the contract
  `Offered`.
- **D-E4-12 Evidence to canon is post-commit and best-effort; attestation from canon is
  same-batch.** Money-safety flows in only one direction; §7.2's "same WriteBatch" wording applies
  to inputs to the journal, not to evidence output. Only `Public` contracts, whose every party
  accepted that visibility, ever reach canon.
- **D-E4-13 `EconomicEvent` payloads carry ids and kinds only** — tighter than §12.4's sketch
  (`TransactionPosted { id, cause }` would leak a staff reason string).
- **D-E4-14 Programmatic resolution first; a graded escalation ladder; stop only the subject, and
  only when a human must decide** (owner decision 2026-09-03, §4.6). A one-staffer game cannot park
  money on "wait for staff" for hours. Downtime and time-jump backlogs are *forgiven* with an
  attributed record (doc 08 §2 forbids charging them anyway); disputes resolve on unanimous party
  agreement; refusals only a human can fix page with the recommended command filled in and suspend
  that obligation alone; the whole-economy freeze stays the only world-stop and now also opens a
  record carrying its diagnosis. This is a standing principle for later economy cuts, recorded in
  08a as decision 25.

## 9. Constants

`SWEEP_PERIOD_SECS` 30 · `MAX_HOLD_OPS_PER_PLAN` 32 · `MAX_ACTIVE_HOLDS_PER_ACCOUNT` 64 ·
`MAX_HOLD_EXPIRIES_PER_SWEEP` 64 · `MAX_OBLIGATIONS_PER_SWEEP` 64 · `MAX_POSTS_PER_SWEEP` 128 ·
`MAX_CATCH_UP_CAP` 8 (default 3) · default `grace.periods` 1, `MAX_GRACE_PERIODS` 254 ·
`SWEEP_GAP_DOWNTIME_SECS` 2 × `SWEEP_PERIOD_SECS` · `MAX_HOLD_EXPIRY_REFUSALS` 3 ·
`MAX_CONSECUTIVE_DEFERS` 16 · `mint_obligation_cap_major` 50 000 (major units, per instrument) ·
`MAX_PARTIES` 16 · `MAX_CONDITIONS` 16 · `MAX_OUTCOMES` 8 (E4b2) ·
`MAX_AMENDMENTS` 32 (E4b2) · `MAX_DEPOSITS` 1 (E4b2, the module's one-custody-op cap) ·
`MAX_AUDIENCE` 32 (E4b2) · `MAX_CONTRACT_NAME_BYTES` 64 (E4b2) ·
`MAX_CONTRACTS_PER_SWEEP` 64 (E4b2) ·
`MAX_DELIVERABLES` 16 · `MAX_SHARES` 32 · `MAX_OPEN_CONTRACTS_PER_ISSUER` 16 ·
`MAX_LIVE_CONTRACTS_PER_PARTY` 32 · `MAX_SETTLE_ATTEMPTS` 8 · `DRAFT_TTL` 7 d · `DISPUTE_TTL` 3 d ·
`MAX_OBSERVER_DEPTH` 4 (E4c) · `MAX_OBSERVER_SETTLEMENTS_PER_EVENT` 8 (E4c fold) ·
`MAX_REPUBLISH_PER_SWEEP` 8 (E4c fold) · `CANON_TAIL` 64 (E4c fold) ·
`STALE_CONTRACT_AGE_SECS` 7 d (E4c fold) · `HISTORY_CAP` 20 (E4c, and `n` is validated to 1..=20) ·
`ESCALATION_WINDOW` 7 d · `ESCALATION_RENOTIFY` 1 h, then daily · `MAX_OBLIGATION_HISTORY` 32 ·
hold/approval/escalation GC retention 7 d after terminal · list cap 20.

## 10. Open questions — resolved by the owner 2026-09-03

1. Sweep period 30 s wall time — **confirmed**.
2. Whether reconciliation stops future periods — **no, not by default.** The owner's constraint:
   "things that stop until staff do something" can wait hours on a game with one staffer who would
   take the alert to an LLM anyway; figure out the best action programmatically and attempt it, and
   only when there is a real issue for a human to decide, page them, escalate, and stop future
   periods. Folded as D-E4-14 / §4.6: automatic forgiveness with a classified cause, unanimous
   party dispute resolution, a graded ladder whose page carries the recommended command, and
   subject-only suspension.
3. Canon evidence bridge in E4c, existence-level only, `Public` contracts only — **confirmed**.
4. *(post-K)* Obligation and contract docs: **conventional module docs, owner-confirmed 2026-09-03**
   ("leave E4 on conventional until K makes it further along and is boot durable"); shaped for a
   mechanical K3 migration (§12).
5. *(post-K)* The one-field `WorldEvent::CanonRecorded { source }` core change (§2) — **accepted**
   (owner, 2026-09-03), an E4c commit.
6. *(post-K)* Whether `mush-economy::post` ever becomes a kernel action provider — **deferred** with
   q4 until K is boot-durable; E4 shapes its records for a mechanical migration either way.

## 12. Doc 09 conformance (post-K, Draft 0.5)

The K series (`codex/k1-contracts`, `fd3108c`; doc 09 K0–K2) landed after this plan was drafted. Doc
09 is a kernel-contract design that E4 overlaps in four places; E4 conforms as follows, and the
K-side facts are cited from `docs/design/09-rules-engine-contracts.md` in that branch.

| Doc 09 | E4 concept | Conformance |
|---|---|---|
| **C8 `ReservationRecord { id, owner, resource, holder: ScopedIdempotencyKey, amount, revision, expires: ClockStamp, state: Held \| Consumed(commit) \| Released \| Expired }`** | `Hold` (§3.2) | A `Hold` **is** the economy's reservation: `Active/Captured{tx}/Released/Expired` map 1:1 onto `Held/Consumed/Released/Expired`; `placed_by` + the placing plan's idempotency key is the `holder`; "acquire, replace, release, and expiry are idempotent authoritative actions … expiry is a causally linked scheduled action and survives restart" is satisfied by hold effects in the journal with deterministic keys and the sweep's `hold/{id}/expire`. Rename the states to doc 09's at build time; add `holder` (the placing key). |
| **§2.5 `ClockRef { Utc, WorldIc{generation}, SceneTurn, DomainTick, Registered{owner,id,generation} }` + `ClockStamp`; C4 "obligations become `ClockProvider` adapters"** | `EconomicClock` / cadence clocks (§3.1) | `EconomicClock` is a **pre-K4 adapter shape**: `RealUtc ≙ Utc`, `WorldIc ≙ WorldIc{generation}`, `Cadence(id) ≙ Registered { owner: economy, id: cadence.<name>, generation }`, `Manual` folds into a registered cadence with a `Manual` source. Deadlines/schedules carry the clock identity so two stamps are comparable only on the same clock (doc 09's rule). When K4 lands, `EconomicClock` becomes a thin alias over `ClockRef` and the sweep registers as the economy's `ClockProvider`; no journal migration (the serde shape stays). |
| **C6 `SemanticEvent { id, cause: CausalRef{root: CommitToken, parent, ordinal}, owner, kind, schema, truth, eligible_witnesses }`** (in K's code today only as the action-local `PreparedCausalRef`/`PreparedEventId`, `mush-core/src/action.rs:2265-2731`; `SemanticEvent`/`WitnessSet` are design vocabulary, zero matches) | `EconomicEvent` (§6.3), `EconomicCause::External(CausalRef)` (E2) | `EconomicEvent` is the **pre-C6 form** of the economy's semantic events (ids + kinds only, D-E4-13) and migrates to `SemanticEvent` when commit tokens exist. **Name collision (prose today, code later):** economy's E2 `CausalRef { provider, kind, id, revision }` (`mush-economy/src/record.rs:278-287`) is *not* doc 09's causal reference; E4 renames it **`ExternalRef`** (serde shape unchanged) and reuses it for `TypedCondition::Fact` instead of minting a `FactRef` — one external-reference type, with `revision` for free. Contract attestations record the canon event's id and `source` today, and its commit-token causal reference once C6 lands, so front attribution nests inside the kernel's causal chain rather than running as a parallel system. |
| **§2.6 canonical hashes "state their domain/encoding version"** | `plan_hash` v3 (§3.3 inv. 27) | The hash stays byte-compatible (E2 pin) and the record gains a `hash_domain: "economy.plan/v3"` note in the journal entry's schema doc, not in the hashed bytes. |
| **C2 / K2 "new authoritative state-family actions must use the prepared path; legacy handlers remain a compatibility risk until migrated"; §1.4 durable atomicity is K3** | `post`, `submit`, E4's new docs | E3's `post`/`submit` are legacy K0-lane handlers by doc 09's own classification, hardened by K's two-phase `PreparedLedgerAppend`. E4 **keeps the journal as authority** (holds as effects — stronger than the K2 bridge's process-local serial, which has no restart hydration) and keeps `obligation:{id}` / `contract:{id}` as conventional container docs, **but** shapes them so the K3 migration is mechanical: one mutation per key per plan, canonical post-images, no fallible step after the first install (already §4.2). Registering them as K2 state families now would put money-adjacent truth on a lane that doc 09 itself says cannot yet hydrate at boot. **Owner decision requested (§10 q4).** |
| **C8 idempotency list: "quotes and contract settlement; production, upkeep, and scheduled obligations"** | keys `contract/{id}/settle`, `obligation/{id}/{period}`, `hold/{id}/expire` | Complies; a conflict on an internal deterministic key is treated as an integrity incident (freeze the smallest scope — here the contract or obligation, via the escalation ladder), matching C8. |

K's own changes inside `mush-economy` (the `prepare_commit` → custody move → `install_commit` split)
are **kept** by E4: `post_with` (§4.1) wraps that sequence, and settlement/sweep posts go through it.
**One K-introduced latent hazard is fixed in E4b1** (found by two of the three K reviews): K's
`install_commit(ctx, commit)?` sits *after* `run_custody_effects` (the world mutation) and can
return `JournalUnavailable`/`StalePrestate` if the allocator or ledger tail moved between prepare
and install — unreachable today (no custody provider or `CarryAttempt` hook can reach the economy
store; the hook has no `Ctx`) but it contradicts the file's own "never `?` after the world moves"
rule and has no test. E4b1 makes the install **infallible** (the two checks become asserted
invariants with the reachability argument recorded), keeping the custody move as the last fallible
act; a regression test drives a stub provider that reenters and proves the assertion, not a desync,
is what fires. `mush-org`'s split already runs its fallible check before any mutation and is the
shape to match.
**K3 review (2026-09-03).** Re-asked at the end of the E4c verify fold, with the
records as built: **do not migrate `obligation:{id}` / `contract:{id}` to K2 state
families yet.** Three concrete blockers, none of them a matter of taste. (1) A K2
bundle carries one state family's write set; E4's terminal steps do not — a settlement
is a **journal append plus companion documents** (the contract, its holds, its
accounts) in one batch, and `post_with_companions` exists precisely because the
evidence and the money it released must be one durable write. (2) `Ctx` has no family
**read** path, so boot's reconciliation pass and the sweep — both of which scan every
stored obligation and contract — would go blind the day the documents moved. (3) The
K3 lane refuses to run behind an unacknowledged legacy seal, and E4's records are
written by legacy K0-lane handlers today. The migration is unblocked by three things,
in this order: **mixed write sets** (a bundle that can carry a ledger append and
companion docs), a **`Ctx` family read path**, and the **seal folded into the bundle**.
It is then still an explicit **owner decision**, not a mechanical port, because the
journal would stop being sole authority for money-adjacent truth — which is the one
property every invariant in §3.3 is anchored to. Recorded alongside it: the
**pre-existing E0 hazard** that a batch dropped deterministically only ever logs an
unsealed journal chunk and the non-ledger module ops, so a lost write is visible as a
gap rather than as an error — E4's contract, obligation and escalation documents ride
that batch and inherit it. Fixing that belongs to K's rebase, with the ledger and the
module ops sealed together.

`mush_core::ScopedIdempotencyKey`/`IdempotencyScope` (`action.rs:374-426`) exist but are ingress/
gateway machinery; the sweep's server-internal deterministic keys stay `IdempotencyKey` (optional
future alignment, not required). `mush-contract-tests` hosts only the toy `focus/brace/guarded`
slice and doc 09 K8 names no economy migration; an economy contract-settlement fixture there is
**backlog**, not a gap. Doc 09 §10 never mentions the economy — whether `post` ever becomes a
kernel action provider is an owner question (§10 q6).

## 11. Review outcome (Draft 0.1 → 0.2)

Three passes ran on 2026-09-03 (Sonnet, during the Opus outage). Folded:

- **Adversarial:** `Capture` gains an explicit `amount` and per-hold attribution (A1);
  `@account/close` refuses with holds/obligations (A2 → inv. 29); `reverse` refuses `hold_effects`
  and boot treats a reversed settlement as `Disputed` (A3 → inv. 28); `Place` refused on
  unbounded-credit accounts (A4); the hash covers the whole consent surface — parties, terms,
  conditions, deadline, visibility, scope (B5); conditions re-verified live at settlement +
  custody locks for `DepositInto` (B6); org accept needs both the capability and debit authority
  (B7); `maximum_catch_up` clamped (C8); grace deadlines typed on the schedule's clock, backwards
  clocks yield zero (C9); the due pool excludes quarantined states (C10); `held_amount` gets its
  own seq anchor (D11); the canon observer is infallible (D12); the sweep uses exhaustive matches,
  never `?` (D13); hold-id floors from the journal (D14); canon evidence only for `Public` contracts
  every party consented to (E15); `@hold/list` list discipline (E16); contract/deliverable caps and
  Draft GC (F17, F18).
- **Grounding:** `mush_org::audit()` does not exist → a five-line wrapper (G1); `EconomicVisibility`
  is a closed enum, not a struct with `public_summary` (G2); one durable gate per world step makes
  per-debtor gating inexpressible → D-E4-6 rewritten, core seam dropped (G3); `post` hard-wires
  `WorldAuth` → `post_with(ctx, plan, auth)` internal entry (G4).
- **Design-fit:** stale-revision tests added (A); `After { condition, delay }` for E6's delayed
  settlement (A); D-E4-2 cites §18-E4 (A); `Reconciliation` justified from §8.2 (C); `Accepted`
  folded → D-E4-10 (C); hold placement vs `ApprovalRule` → D-E4-11 (C); evidence post-commit →
  D-E4-12 (E); success notices suppressed, transitions notify (F); the sweep's contract step
  belongs to E4b2, `Fact` inert until E4c (G); hidden-party tests for holds/obligations (G);
  event payload tightening → D-E4-13 (E); fixed-per-period amounts flagged for E5 (A).
- **Not folded (deliberate):** widening `EconomicVisibility` to a `DisclosureLevel` struct (§12.2's
  sketch) — the closed enum suffices for E4's `Public`-only bridge.
