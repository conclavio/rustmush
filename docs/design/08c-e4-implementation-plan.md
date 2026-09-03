# E4 implementation plan — holds, escrow, obligations, and contracts

**Status:** Draft 0.1 — pre-build plan, awaiting the three-pass design review (adversarial
money-safety, code-grounding, design-fit). Companion to
[08-economy-organizations-ownership.md](08-economy-organizations-ownership.md) (the design; §8, §12,
§18-E4), [08a-economy-build-status.md](08a-economy-build-status.md) (the ledger + the E4 pickup), and
[08b-e3-implementation-plan.md](08b-e3-implementation-plan.md) (E3 as built — E4 extends it).
Grounded in the tree at `9d5ea1d`; every `file:line` was checked by survey. Departures from doc 08
are numbered decisions (D-E4-n, §8) so the review can strike them.

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

- **`EconomicClock::Registered { provider, id }`.** No clock-provider seam exists anywhere
  (survey), and no consumer needs one before E7's campaign clocks. E4 ships `RealUtc`, `WorldIc`,
  `Manual`; the enum stays open.
- **Ruleset consequence policies** (eviction windows, lifestyle strain, loyalty) — E5 hooks onto
  the `ObligationChanged` event; E4 stops at `Grace → Delinquent → Reconciliation` state.
- **Work orders / production reservations** (E7), **markets/quotes/fences** (E6/E8), **fee and tax
  templates**, **interest**.
- **Settlement conditions that react to LLM output or narrative prose** — never (§13). And
  conditions that react *synchronously to a `WorldEvent` for money*: the survey found no event
  source that is stably identified, immutable, and durable at event time (§4.6), so settlement rests
  on explicit attestations and the economy's own journaled facts (D-E4-4).
- **Per-obligation or per-contract scheduler entries.** The core scheduler is in-RAM, re-armed from
  scratch at boot, and only `Scene` ticks are dynamic (`mush-core/src/module.rs:208-220`,
  `engine.rs:503-522`); everything runs on one bounded economy sweep (D-E4-3).
- **A durability checkpoint** (08a decision 12) — still deferred; E4 adds journal volume (holds,
  obligations) and is the point to *measure* boot cost, not to build the checkpoint.
- **Player-created orgs**, **`PolicyRef`**, **per-account isolation of the whole-economy freeze**.

---

## 2. Cut structure

Five commits in four named cuts, each independently CI-green and **each adversarially verified
before the next builds on it** (08a's recommendation after E3's single end review):

| Cut | Crate(s) | One line | Verify axis |
|-----|----------|----------|-------------|
| **E4a** | `mush-economy` (pure) | `Hold`/`HoldOp`/`HoldEffect`; available-balance enforcement; the clock/schedule/catch-up math; `Obligation` + `Contract` records and their pure state machines; the deterministic split; `ContractAuth`; `plan_hash` v3 | double-spend, rounding, catch-up arithmetic |
| **E4b1** | `mush-core` (one small seam, own commit) + `mush-economy` module | holds + obligations live: persistence, journal-derived `held_amount`, the **economy sweep** tick, IC-clock plumbing, `@hold/*`, `@obligation/*`, `Ctx::gate_on_durable_ack_for` | duplicate fires, restart, time jumps |
| **E4b2** | `mush-economy` module | contracts end to end: draft/offer/accept (escrow at accept)/attest/settle/dispute/cancel/expire, `Lease`/`Collateral`, `@contract/*` | consent forgery, settle-twice, hidden parties |
| **E4c** | `mush-economy` + `mush-org` + `mush-jobs` + `mush-server` glue | the `FactProvider` seam (jobs implements it), the `CanonRecorded` attestation observer, `EconomyObserver` events + the canon evidence bridge, approvals → org audit, `@econ/history` under the disclosure filter | existence leaks, evidence leaks |

Dependency directions: `mush-economy → {core, org}` unchanged (it subscribes to core's
`CanonRecorded` event; it never depends on canon). New edges: `mush-jobs → mush-economy` (E1c
direction — the fact provider lives with the data), and `mush-server` implements the canon bridge
(the composition root already depends on everything). `mush-scenes` gains nothing: a scene concludes
by committing canon (`mush-scenes/src/lib.rs:282-311`), which the canon observer sees.

---

## 3. E4a — the pure library

### 3.1 Ids and clocks (`ids.rs`, new `clock.rs`)

`int_id! { HoldId }`, `int_id! { ObligationId }`, `int_id! { ContractId }`.

```rust
pub enum EconomicClock { RealUtc, WorldIc, Manual }          // Registered{..} deferred (D-E4-2)
pub struct EconomicDeadline { pub clock: EconomicClock, pub at: i64 }
pub struct EconomicSchedule {
    pub clock: EconomicClock,
    pub next_due: i64,                // absolute, on `clock`
    pub cadence: Option<i64>,         // None = one-shot
    pub maximum_catch_up: u8,         // periods settled per sweep before Reconciliation (default 3)
    pub manual_now: i64,              // the Manual clock's position (advanced only by an explicit act)
}
/// Pure: how many periods are due at `now`, and the schedule after settling `n` of them.
pub fn due_periods(s: &EconomicSchedule, now: i64) -> u32;
pub fn advance(s: &EconomicSchedule, n: u32) -> EconomicSchedule;
```

`ValidateEnv` gains `ic_now: i64` beside `now` (the module feeds `w.now()` and
`w.game_now_unix()`, `mush-core/src/world.rs:299-387`); a `Clocks { real, ic }` helper resolves a
deadline or schedule on its clock, `Manual` from the record's own `manual_now`.

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
    pub status: HoldStatus,                      // Active | Captured { transaction } | Released | Expired
    pub last_journal_seq: u64,
    pub revision: u64,
}
pub enum HoldOp {
    Place   { account: AccountId, amount: Amount, beneficiary: Option<PrincipalId>, purpose: HoldPurpose, expires: Option<EconomicDeadline> },
    Capture { hold: HoldId },                    // the plan's own lines carry the debit; see invariant 24
    Release { hold: HoldId },
    Expire  { hold: HoldId },                    // sweep-only (cause Internal { kind: "hold.expire" })
}
```

`EconomyPlan` gains `hold_ops: Vec<HoldOp>` (≤ `MAX_HOLD_OPS_PER_PLAN` 32, ≤ 1 per hold) and
`expected_hold_revisions`. `EconomicTransaction` gains `#[serde(default)] hold_effects:
Vec<HoldEffect { hold: Hold /* full post-state */, kind }>`. `Prepared` carries them.

**Holds are journal-authority** (D-E4-1): placing, capturing, releasing, and expiring are effects
inside journal transactions (a `Place`/`Release`/`Expire` is an effect-only entry, legal under E3's
invariant 10); `hold:{id}` docs are pure projections; **`Account.held_amount` is re-derived at boot
as the sum of the account's `Active` holds** and healed/frozen with the same directional rules as
`posted_balance`.

### 3.3 Invariants (extending E3's 10–22)

23. **Available balance.** A debit line's post-state must satisfy `posted' − held' ≥ floor`
    (`CreditPolicy` evaluates *available*, not posted). `Place` counts toward `held'` in the same
    plan; a `Capture` of hold H **releases H's amount from `held'` before the check** and requires
    the plan to carry a net debit on H's account of at least… no: of **at most** `H.amount` plus
    the account's remaining available (a capture may consume less than the hold; the rest is
    released — the effect records `captured: Amount`).
24. `Capture`/`Release`/`Expire` require the hold `Active`; `Expire` requires the deadline passed on
    its clock and is only legal with cause `Internal { kind: "hold.expire" }` (the sweep).
25. Authorization: `Place` needs **debit authority** on the account (it is a promise to pay —
    `authorize(actor, account, −amount)`); `Capture` needs debit authority on the account **or** is
    an internal settlement whose contract acceptances cover it (`ContractAuth`, §3.5); `Release`
    needs the placer's authority, the beneficiary's (releasing their own claim), or staff.
26. Caps: `MAX_ACTIVE_HOLDS_PER_ACCOUNT` (64); a hold's `amount` ≤ the account's available at
    placement (a hold cannot promise money that is not there — no negative available).
27. `hold_ops` join the canonical hash under skip-if-empty with a total order
    `(hold-or-account id, discriminant, canonical bytes)`; an E3-shaped plan hashes byte-identically
    (new hard-coded digest pin beside the E2 one).

### 3.4 Obligations (`obligation.rs`, pure state machine)

```rust
pub struct Obligation {
    pub id: ObligationId,
    pub debtor: PrincipalId, pub creditor: PrincipalId,
    pub debit_account: AccountId, pub credit_account: AccountId,
    pub amount: Amount,                          // fixed per period in E4 (ObligationAmount enum stays open)
    pub schedule: EconomicSchedule,
    pub grace: GracePolicy { pub periods: u8 },  // missed periods tolerated before Delinquent
    pub status: ObligationStatus,                // Active | Grace { missed, deadline } | Delinquent | Suspended | Reconciliation { missed } | Satisfied | Cancelled
    pub cause: ObligationCause,                  // Contract(ContractId) | Staff { reason } | Ruleset { namespace }
    pub periods_settled: u32,
    pub revision: u64,
}
/// Pure sweep step for one obligation at `now`: which period indexes to attempt (≤ maximum_catch_up),
/// and the status if `k` of them exceed the cap (→ Reconciliation) — no money logic here.
pub fn plan_sweep(o: &Obligation, now: i64) -> SweepIntent;
/// Pure transition on a settlement outcome (Posted | InsufficientAvailable | Refused).
pub fn advance(o: &Obligation, period: u32, outcome: SettleOutcome, now: i64) -> Obligation;
```

Each period's settlement is an ordinary money plan with `cause: Internal { kind: "obligation", id }`
and **idempotency key `obligation/{id}/{period}`** — so a duplicate sweep fire, a retry, or a
restart mid-sweep resolves to `AlreadyPosted` (the exit criterion's "duplicate scheduler events").
A failed period → `Grace { missed: 1, deadline }`; further failures increment `missed`; past
`grace.periods` → `Delinquent` (no further attempts until staff/ruleset act); more due periods than
`maximum_catch_up` → `Reconciliation { missed }` (§8.2: "excess periods become an explicit
staff-visible reconciliation state, not an unbounded loop"). `Suspended` is staff. Obligation
status is **doc-authority** (`obligation:{id}` container); its settlement history is the journal.

### 3.5 Contracts (`contract.rs`, pure)

```rust
pub struct EconomicContract {
    pub id: ContractId,
    pub issuer: PrincipalId,
    pub parties: BTreeSet<PrincipalId>,          // ≤ MAX_PARTIES (16), includes issuer
    pub terms: ContractTerms,
    pub terms_hash: [u8; 32],                    // canonical; acceptances and attestations bind to it
    pub acceptances: Vec<Acceptance { party, at }>,
    pub conditions: Vec<ConditionState { condition: TypedCondition, satisfied: Option<Attestation> }>,
    pub escrow: Vec<HoldId>,
    pub deadline: Option<EconomicDeadline>,
    pub state: ContractState,                    // Draft | Offered | Active | ReadyToSettle | Settled { transaction } | Disputed | Cancelled | Expired
    pub visibility: EconomicVisibility,
    pub scope: Option<OrgScopeId>,               // for capability-based attest/dispute on org contracts
    pub revision: u64,
}
pub struct ContractTerms {
    pub consideration: Vec<Consideration { from: PrincipalId, account: AccountId, amount: Amount }>,   // escrowed at accept
    pub deliverables: Vec<Deliverable>,          // TitleTransfer { object, from, to } | DepositInto { object, vault } | Collateral { object, from } | Obligation { template }
    pub split: Vec<PayoutShare { to: AccountId, share: Share }>,   // Share::Exact(Amount) | Share::Permille(u32)
    pub remainder_to: AccountId,
    pub on_expiry: Expiry,                       // Refund | Forfeit { to: AccountId } | Dispute
}
pub enum TypedCondition {
    Attestation { by: AttestorSet },             // Party(PrincipalId) | Capability(scope) | Staff
    DepositInto { object: ObjId, vault: VaultId },        // the economy's own journaled custody effect
    TitleHeld  { object: ObjId, by: PrincipalId },       // journal-authority title state
    Fact       { fact: FactRef { provider: SmolStr, kind: SmolStr, id: u64 } },   // via FactProvider (E4c)
    Before     { deadline: EconomicDeadline },           // time
}
/// Pure: the settlement plan for a ReadyToSettle contract — captures every escrow hold, applies the
/// deterministic split (permille floors; remainder to `remainder_to`), the deliverables' title ops,
/// collateral release; cause Internal { kind: "contract", id }, key `contract/{id}/settle`.
pub fn settlement_plan(c: &EconomicContract, snapshot: &EconomySnapshot) -> Result<EconomyPlan, ContractError>;
/// Pure: refund plan (cancel/expiry-refund) — release every escrow hold; key `contract/{id}/refund`.
pub fn refund_plan(c: &EconomicContract) -> EconomyPlan;
```

**Consent (D-E4-4).** `@contract/accept` runs **under the accepting party's live authority** and,
in the same step, places that party's consideration hold (invariant 25 — debit authority). An
acceptance is recorded against `terms_hash`; changing terms after any acceptance is refused (a new
contract is drafted). When every party has accepted the contract becomes `Active`. **Settlement
authority is the set of acceptances**: `ContractAuth` (an `AuthEval` the pure engine takes for
internal plans with `cause: Internal { kind: "contract", id }`) authorizes exactly the captures,
title transfers (both sides — the source and destination parties both accepted, D-E3-12 holds),
and collateral releases the terms describe, and nothing else. No authority is stored; the
acceptance *is* the authority, bound to the hash, re-checked at settlement against the live record.
Staff may settle a `Disputed` contract either way.

**Conditions** are satisfied only by durable facts: an explicit `@contract/attest` (the actor must be
in the `AttestorSet`; recorded with actor + time), a journaled `Deposit` into the named vault
(checked against the journal-derived last-custody map), journal-authority title, a `FactProvider`
answer (`Some(true)`; `None` = unknown = unsatisfied), or the clock. **Settlement is a deliberate,
idempotent post**: the step that satisfies the last condition (attest/deposit) moves the contract to
`ReadyToSettle` and settles in the same step; deadline-only readiness is settled by the sweep.
Settling twice is `AlreadyPosted` (scenario 16).

**Deterministic split** (scenario 17): `Exact` shares first, then `Permille` shares of the remaining
escrowed total, floored; the remainder goes to `remainder_to`; the split must be feasible
(`Σ Exact ≤ total`, `Σ Permille ≤ 1000`) at *draft* time and is re-checked at settlement.

### 3.6 E4a tests (`tests/e4a.rs`, ~40)

Holds: place/capture/release/expire on hand-built snapshots; available enforcement — two holds that
together exceed available refuse the second (scenario 13 pure half); a debit that ignores a hold is
refused; partial capture releases the rest; capture of a non-Active hold refused; expire without a
passed deadline refused; caps. Schedule math: `due_periods` across a downtime gap and an IC jump;
`advance`; `plan_sweep` yields ≤ `maximum_catch_up` and flags Reconciliation; grace/delinquency
transitions. Contracts: acceptance binds to `terms_hash`; terms change after acceptance refused;
`ContractAuth` authorizes exactly the terms' captures/transfers and refuses an extra line or an
unlisted object; `settlement_plan` split with permille floors + remainder (scenario 17 pure half);
infeasible split refused at draft; `refund_plan`. Hash v3: E3-shaped plan == hard-coded digest;
hold-op order independence.

---

## 4. E4b1 — holds and obligations live

### 4.1 One core seam (own commit)

`Ctx::gate_on_durable_ack_for(actor: ObjId, strict: bool)` — `Deferred::DurableGate` already carries
`actor` (`mush-core/src/engine.rs:215-228`); today `gate_on_durable_ack*` hard-wire `ctx.actor`
(`module.rs:1848-1859`), and a tick runs as `GOD` (`engine.rs:733-743`). The sweep must gate the
**debtor's/creditor's** "rent paid" notice on the durable ack (invariant 9), not GOD's. Ten lines
plus a core test; the existing methods delegate to it.

### 4.2 Persistence + boot

| Doc | Polarity | Boot rule |
|-----|----------|-----------|
| `hold:{id}` | projection | derived from the journal's last `HoldEffect` per hold; lag heals; disagree-at-claimed-seq / ahead → freeze economy; missing doc → rebuilt silently (no latch to lose); unjournaled doc → not loaded, warn |
| `Account.held_amount` | projection | re-derived = Σ `Active` holds on the account; stored value compared like `posted_balance` (drift → freeze) |
| `obligation:{id}` | container | corrupt → fail boot; debtor/creditor object missing or `GOING`, or either account missing → `Suspended` + re-persist (never delete); a `Contract`-caused obligation whose contract is not `Settled`/`Active` → `Suspended` |
| allocator floors | | `next_hold_id ≥ max(hold_effects.hold.id)+1`; `next_obligation_id ≥` every parsable `obligation:` key; meta high-water marks, persisted at boot end (the E3 fold rule) |

`commit_prepared` persists every touched `hold:{id}` projection and the touched accounts (whose
`held_amount` moved). Latch rule unchanged: a doc-authority write (obligation status) touches no
journal-authority field.

### 4.3 The economy sweep (D-E4-3)

One `TickCadence::Coarse(SWEEP_PERIOD)` tick, tag `economy.sweep` (`Registrar::tick`,
`module.rs:1394-1399`). Per fire, in order, **all no-ops while the economy is frozen**:

1. **Expire holds** whose deadline passed on their clock — effect-only posts, cause
   `Internal { kind: "hold.expire", id }`, key `hold/{id}/expire`; ≤ `MAX_HOLD_EXPIRIES_PER_SWEEP`.
2. **Settle due obligations**, oldest `next_due` first, ≤ `MAX_OBLIGATIONS_PER_SWEEP` (64)
   obligations and ≤ `MAX_POSTS_PER_SWEEP` (128) posts per fire: for each, `plan_sweep` → attempt
   each period as a post with key `obligation/{id}/{period}`; `Posted`/`AlreadyPosted` → `advance`;
   `CreditViolation` (insufficient **available**) → `Grace`/`Delinquent`; any other refusal →
   `Suspended` + staff log; periods beyond the cap → `Reconciliation`. Notices to debtor and
   creditor via `out.to_actor` gated with `gate_on_durable_ack_for(debtor, false)`.
3. **Contracts**: expire `Offered`/`Active` contracts past their deadline (`on_expiry` → refund /
   forfeit / dispute), settle `ReadyToSettle` ones whose readiness came from a `Before` condition.
4. **GC** expired approval requests (E3's rule) and `Released`/`Expired` holds older than a
   retention window (their history stays in the journal).

Catch-up is computed from `next_due` and the clock, **never from tick count** — the
`mush-sim/src/route.rs:14-20, 186-208` pattern. The first sweep after a restart fires immediately
(`engine.rs:447-453`) and runs catch-up under the caps; a large IC jump lands in `Reconciliation`,
not a charge storm; work left over waits for the next fire (§14.3: bounded per step).

### 4.4 Commands

```text
@hold/place <account>=<amount> [for=<principal>] [expires=<secs|ic:<secs>>] [reason]   (debit authority)
@hold/release <id>    @hold/capture <id>=<to account> [<amount>]    @hold/show <id>   @hold/list [account=]
@obligation/create <debtor acct>=<creditor acct> <amount> every=<secs|ic:<secs>> [start=] [grace=<n>] [catchup=<n>]  (staff / ruleset)
@obligation/show|list|suspend|resume|cancel <id>   @obligation/advance <id>  (Manual clock)   @obligation/reconcile <id>=<settle <n>|forgive>  (staff)
```

All money through `post`; every refusal through the E3 existence-hiding renderer (a hold or
obligation the actor may not view renders as "No such …"; view = account view on either side, or
beneficiary/party, or staff).

### 4.5 E4b1 tests (`tests/e4b1.rs`, ~25)

**13** two holds racing the same available refuse the second through the live seam; a payment that
would dip into held funds is refused; **14** capture, release, and expiry each: `fold_kv` + `reboot`
→ identical `held_amount` and hold state; `fire_due` twice for the same expiry → one effect; a
restart between the sweep's post and its doc write converges; **15** an obligation settles on
schedule (set_now steps), fails into `Grace` when available is short, reaches `Delinquent` after
`grace.periods`, and after a simulated downtime of ten periods with `maximum_catch_up = 3` settles
exactly three and lands in `Reconciliation`; an IC-ratio jump does the same; `fire_due` twice at
the same instant posts once per period (duplicate scheduler events); the whole-economy freeze stops
the sweep; the debtor's notice rides the gate for the debtor (core test). Plus the boot quartet for
`hold:{id}` and the `held_amount` drift freeze.

---

## 5. E4b2 — contracts live

Persistence: `contract:{id}` **container** docs (corrupt → fail boot); boot reconciles each
non-terminal contract against the journal: an `Active` contract whose escrow hold is not `Active`
in the journal → `Disputed` + loud log (§15.2 "disable and quarantine, reserve no value");
`Settled { transaction }` whose transaction is absent → freeze the economy (a settlement the journal
does not know is drift). `next_contract_id` floored on every parsable key + meta.

Commands (party-gated unless noted; org contracts also accept `contract.*` capabilities on the
contract's scope — new vocabulary entries `contract.draft`, `contract.attest`, `contract.dispute`):

```text
@contract/draft <name> [scope=]            → Draft (issuer); then terms builders:
@contract/party <id>=<principal>   @contract/pay <id>=<from acct> <amount>   @contract/deliver <id>=title <obj> <from>→<to> | deposit <obj> <vault> | collateral <obj> <from>
@contract/split <id>=<acct> <exact|N‰> …  @contract/remainder <id>=<acct>   @contract/condition <id>=attest <by…> | deposit <obj> <vault> | title <obj> <by> | fact <provider> <kind> <n> | before <deadline>
@contract/deadline <id>=<deadline> [refund|forfeit=<acct>|dispute]
@contract/offer <id>                       Draft → Offered (freezes terms_hash)
@contract/accept <id>                      a party; places that party's consideration hold under their live authority
@contract/attest <id>=<n>                  an attestor; may settle in the same step
@contract/dispute <id>   @contract/cancel <id>   @contract/resolve <id>=settle|refund   (staff)
@contract/show <id>   @contract/list
```

`Encumbrance::Lease { lessor, contract }` / `Collateral { contract }`: placed at activation for
`Collateral`/`Obligation` deliverables (the source party's acceptance authorizes it), released at
settlement/refund. A lease is a contract whose deliverable is an `Obligation` template: activation
creates the obligation (`cause: Contract(id)`) and the `Lease` encumbrance; cancellation/expiry
cancels the obligation.

Tests (`tests/e4b2.rs`, ~25): **16** a Shadowrun run — Johnson drafts, three runners + Johnson
accept (Johnson's escrow hold placed at his accept), the run's condition is attested, settlement
posts the split once; a second attest / a second settle is a HIT; the split's remainder lands where
declared (**17** live half); a party who did not accept cannot be debited; terms mutation after an
acceptance refused; settlement refuses an extra line (`ContractAuth`); a sale — buyer pays, seller's
title transfers, both accepted, both authorities re-checked live (revoke the seller's title before
settlement → `Disputed`, nothing moves); expiry → refund / forfeit; dispute freezes escrow; reboot
at every state; **hidden parties**: a non-party's `@contract/show` and every refusal render the
nonexistent-id text; a party sees the other parties only as the terms name them.

---

## 6. E4c — facts, events, evidence, the disclosure filter

1. **`FactProvider` seam** (economy-defined, E1c-shaped: `provider() -> &str`,
   `resolve(&self, w, kind, id) -> Option<FactState { satisfied: bool, actor: Option<ObjId>, at: i64 }>`,
   frozen via `EconomyModule::with_facts(..)`). `mush-jobs` implements `JobsFactProvider` for
   `kind = "resolved"` **only for payload-bearing jobs** (a payload-less job can be reopened,
   `mush-jobs/src/commands.rs:583-593` — not a terminal fact); `mush-jobs → mush-economy` is a new,
   acyclic edge.
2. **Canon attestation observer**: economy subscribes to `WorldEventKind::CanonRecorded` (core's
   event, `mush-core/src/module.rs:388-422`; the only production emitter is
   `mush-canon/src/lib.rs:159`, which is documented as terminal in its command) and, in the same
   step — hence the same `WriteBatch` as canon's own writes — records an `Attestation { source:
   canon:{id} }` on every `Active` contract condition `Attestation { by: Canon { kind, entities ⊇ } }`
   that matches, settling if that completes the contract. Canon's RAM-tail-only queryability
   (`CanonState::recent`) does not matter: the attestation is durable on the contract doc.
3. **`EconomyObserver` seam + compact events** (§12.4): `EconomicEvent::{TransactionPosted, HoldChanged,
   ContractChanged, ObligationChanged, AssetTitleChanged}` with **ids and kinds only** — no amounts,
   no parties — delivered to observers frozen at wiring. `mush-server` implements the **canon bridge**:
   for a settlement or obligation transition whose `visibility.public_summary` is set, commit a
   `CanonDraft { kind: "economy.settled" | "economy.delinquent", entities: parties, scope by
   DisclosureLevel, detail: existence-level only }` — the composition root depends on both crates;
   economy still depends on neither canon nor jobs.
4. **Approvals → org audit** (E3 leftover): `mush_org::audit(ctx, OrgAction)` (a small public fn
   over `OrgStore.audit`/`AUDIT_LOG`, `mush-org/src/api.rs:35,130`); `@econ/approve|deny` append an
   entry naming request id and account scope.
5. **Disclosure filter, first cut**: `@econ/history <account> [n]` for non-staff — the journal
   entries touching an account the actor may view, rendered per `EconomicVisibility`: the viewer's
   own side in full; the counterparty named only when the viewer may view that account too, else
   "another account"; `Staff`-visibility entries omitted entirely (not "hidden"); descriptions
   shown only to parties. `EconomicVisibility` is thereby **enforced for reads**; recording is
   unchanged. `@contract/show` and `@hold/show` render through the same rule.

Tests (`tests/e4c.rs` + one in `mush-jobs`, ~15): a payload job's approval satisfies a `Fact`
condition and a payload-less job's does not; a canon commit attests and settles in one batch
(`drain_write_batch`); events carry no amounts; the canon bridge emits only for `public_summary`
and only existence-level detail; `@econ/history` hides counterparties the viewer may not view and
omits staff-visibility entries; approve/deny land in the org audit ledger.

---

## 7. Build discipline

Per cut: (1) confirm the cut's slice of this plan against the tree; (2) build, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo test --workspace`; (3) **a probe-confirming
adversarial verify of that cut** (three lenses, throwaway probe tests) before the next cut builds
on it; (4) fold with regression tests; (5) commit with the verify outcome; nothing pushed until the
owner says so. `cargo fmt` stays off until the pending reformat lands.

## 8. Decisions for the design review

- **D-E4-1 Holds are journal-authority effects; `held_amount` is a re-derived projection.** Same
  argument as D-E3-1: a companion-doc drop must never let held value evaporate or double-count. A
  hold has no recovery latch, so a missing doc heals silently. *Alternative:* holds as container docs
  reconciled against the journal — rejected, it reintroduces a second truth for available balance.
- **D-E4-2 Three clocks, `Registered` deferred, `Manual` advanced only by explicit act.** No provider
  seam exists; E7 builds it with its consumer. IC time enters the engine as `ValidateEnv.ic_now`.
- **D-E4-3 One bounded sweep tick; catch-up from schedule arithmetic, never from tick count;
  per-period idempotency keys.** The core scheduler is in-RAM, re-armed at boot, fires each tick
  exactly once regardless of downtime, and cannot host per-record timers. Excess periods →
  `Reconciliation`, not charges. Per-sweep caps make one world step bounded.
- **D-E4-4 Contracts are the consent carrier; acceptance bound to `terms_hash` is the settlement
  authority, re-checked live; conditions settle on attestations and the economy's own journaled
  facts.** No stored authority; no synchronous money on foreign events. The canon observer records
  a durable attestation in canon's own batch rather than trusting canon's RAM tail.
- **D-E4-5 Obligation and contract *state* are doc-authority containers; their money is the
  journal.** Boot reconciles state against journal facts and quarantines (`Suspended`/`Disputed`)
  rather than guessing; a `Settled` contract with no journal transaction freezes the economy.
- **D-E4-6 A tick gates the debtor's notice, not GOD's** — one small core seam
  (`gate_on_durable_ack_for`), the E2b-strict-flag precedent.
- **D-E4-7 The disclosure filter starts as a read-side renderer over the journal**, enforcing
  `EconomicVisibility` on `@econ/history` and contract/hold views; the evidence bridge lives in the
  server crate so economy depends on neither canon nor jobs.
- **D-E4-8 `Encumber` stays staff-only for manual liens; `Lease`/`Collateral` are contract-placed.**
- **D-E4-9 Available-balance enforcement replaces posted-balance enforcement everywhere** (invariant
  23); a hold can never exceed available at placement.

## 9. Constants

`SWEEP_PERIOD_SECS` 30 · `MAX_HOLD_OPS_PER_PLAN` 32 · `MAX_ACTIVE_HOLDS_PER_ACCOUNT` 64 ·
`MAX_HOLD_EXPIRIES_PER_SWEEP` 64 · `MAX_OBLIGATIONS_PER_SWEEP` 64 · `MAX_POSTS_PER_SWEEP` 128 ·
default `maximum_catch_up` 3 · default `grace.periods` 1 · `MAX_PARTIES` 16 · `MAX_CONDITIONS` 16 ·
`MAX_SHARES` 32 · hold/approval GC retention 7 days after terminal.

## 10. Open questions for the owner (defaults chosen)

1. Sweep period 30 s wall time — confirm (obligations on `WorldIc` still resolve per sweep).
2. Whether `Reconciliation` should also stop *future* periods (default: yes, until staff
   `@obligation/reconcile`).
3. Whether the canon evidence bridge belongs in E4 or waits for E8's dashboards (default: E4c,
   existence-level only).
