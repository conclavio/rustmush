# E6 implementation plan — the Shadowrun run-and-pay economy

**Status:** Draft 0.3a — 2026-09-04; **E6a is built** (§2.1 step one and its verify fold, §3.7 the
ruleset half — 2026-09-05). Re-grounded at `ce43714` after the E1–E5 look-back (08g §11
item 8, DL-02); DL-01 Option A′, DL-10's rolling budget and E2-C16's `InstrumentDisplay` are E6a
economy items. **Prefer §2.1 and §3.7 over §3's pre-build text wherever the two differ.**
Companion to [08-economy-organizations-ownership.md](08-economy-organizations-ownership.md) (§9.4, §11.2, §14.2,
§18-E6), [08a-economy-build-status.md](08a-economy-build-status.md) (the ledger and the E4 seams as
shipped), [08c-e4-implementation-plan.md](08c-e4-implementation-plan.md) (**E4 as built** — prefer
its §3.5a/§4.7/§5.1/§6.1 blocks over §3–§6's pre-build text), and
[08d-e5-implementation-plan.md](08d-e5-implementation-plan.md) (the structure copied here). **The
binding constraints are the owner's twenty-four answers in
[08e-e6-owner-questions.md](08e-e6-owner-questions.md) §3** — restated in §10, cited by number
throughout, overriding doc 08 §11.2 where the two differ — plus doc 10 §12–§15 where it names E6.
Grounded in the tree at `34a53c0` (main, K3 merged) and re-checked at `ce43714` for 0.3; every
`file:line` was checked. Departures are numbered decisions (D-E6-n, §8).

> **Numbering.** The briefing's shorthand drifts one place from 08e §3 between answers 11 and 23.
> **08e §3's numbering is authoritative** and is what this document cites: 11 vendors/quotes,
> 12 clinics, 13 fence pricing, 14 `report_stolen`, 15 delayed settlement, 16 config, 17 chargen,
> 18 migration, 19 cuts + no team vaults, 20 `mush-market`, 21 party, 22 roles, 23 LLMs, 24 board.

> **Gates.** E6a and E6b are **unblocked** — E4b2 (`e56c2a9`, fold `4972455`) and E4c (`8873863` +
> fold) shipped. E6c gates on doc 10 §14's **safe market projection**, not the belief engine.
> Re-ground §3–§5's E4 references against the built economy before each cut starts.

---

## 1. Scope — what E6 is, in the owner's terms

Shadowrun is "much more materially economic than WoD" (08e §3 preamble). E6 makes a runner's money
exact and their work contractual, on top of E4 as built. §10 has the answers; this is what E6 builds:

- **Exact personal money** — `sr.nuyen` at scale 0, the account authoritative after chargen approval
  (answer 1), **karma untouched** (answer 2). Today `nuyen` is a plain sheet number
  (`crates/ruleset-shadowrun/src/schema.rs:298`, `0..=50_000_000`, default 0) read in one place, the
  chargen Resources ledger (`.../src/chargen.rs:351`).
- **Lifestyle as an E4 obligation on an explicit `sr.month` cadence**, autopaid, **at most one
  downgrade per unpaid period** and none for a downtime or clock-jump backlog (answer 3). Today
  `lifestyle` is a cosmetic enum (`.../src/data.rs:225-232`, field `schema.rs:148-152`) with no cost
  anywhere in the crate.
- **Runs as contracts with real principals** — players, authorized org members, staff, player STs and
  campaign systems, **never an uncontrolled faucet**, with author / payer / representative /
  facilitator / adjudicator as distinct *fields* — distinct people only where a conflict of interest
  exists (answers 4, 22; the roles rule in §3.3).
- **An ephemeral party** that is not an organization, not a principal, and owns no account or vault
  (answers 5, 19, 21); acceptance freezes the roster and the exact shares (answers 7, 8).
- **Typed settlement, never prose** — sealed scene outcome, authorized attestation, deterministic
  condition, or composite, all through E4's one `TypedCondition` alphabet, with the adjudicator and
  permitted evidence named in the offer before anyone accepts (answer 6); E6a itself settles by
  attestation alone (D-E6-20), the other routes arrive with E6b. An LLM proposes; it cannot
  invent funds, alter frozen terms, or release escrow (answer 23).
- **A neutral `mush-market`** (answer 20): catalog policy over a small source-controlled starter
  catalog with versioned packs (answers 9–10), vendors and quotes with a declared expiry clock and
  engine-resolved availability tests (answer 11), and a safe public projection.
- **Fences that never launder title** (answer 13): title ≠ possession, a typed `report_stolen`
  (answer 14), and a delayed consignment that **escrows before it takes custody** (answer 15).
- **`[shadowrun.economy]`** (answer 16), chargen shopping as one live cash authority (answer 17), and
  an idempotent migration from `system:sr.migration` (answer 18).

**Exit criterion (doc 08 §17.5/§18-E6):** scenarios 25 and 26 pass — offer, escrow, complete, split
and settle a run under success *and* failure; quote and accept a fenced-goods sale requiring real
delivery and exposing only observer-authorized provenance — plus scenario 29 (an LLM proposal delayed
past quote or policy revision is refused deterministically).

### 1.1 Explicitly out of E6

- **The belief/knowledge engine.** `Claim`/`Belief`/`Evidence` are design-only (08e §1); doc 10 §14
  gives E6c the **safe market projection** instead. Belief-sensitive fence pricing, **heat, notoriety,
  investigations and title laundering** — answer 19's E6c list — therefore move **past E6**, because
  the safe projection supplies no beliefs (§11 q3).
- **Consent by authorship (DL-01 Option A′).** An amendment authored by a party never voids that
  party's own acceptance; every other acceptance is voided and refunded as E4b2 does today. With only
  the payer accepted, `+run/crew accept` amends the roster and payout rows without moving the escrow.
  Full per-party binding stays out of E6 (08g §12.1 item 1; fed back in §7).
- **TacSim industry** (`mush-industry`, deposits, recipes, capacity, the C8 coordinator) and
  heterogeneous reservations — doc 10 §14's boundary puts them in E7; E6's holds stay money-only (T4).
- **Clinics beyond an atomic installation record** (answer 12). E6b ships the augmentation record and
  the rule that payment never edits the sheet; the ruleset-owned installation action that revalidates
  Essence, prerequisites, custody, clinic capability and sheet revision atomically is later. Buying a
  formula or a teacher never bypasses the Karma cost.
- **Owner abandonment** (`title_owner: Option<PrincipalId>`) — 08c §5.x keeps it in E7 with the
  mechanical recipe written out.
- **Team vaults and party-owned money** (answers 5, 19, 21): a persistent crew is a staff-minted
  organization with an account and a vault (08a decision 1).
- **Receipt-based reimbursement** (answer 7's "once attestations are mature"):
  `ExpenseTreatment::Reimbursed` stays declarative; E6 ships a fixed advance.
- Nothing here touches `ruleset-wod` or `mush-housing` (E5).

---

## 2. Cut structure

| Cut | Crate(s) | One line | Verify axis |
|-----|----------|----------|-------------|
| **E6a** | `mush-party` (new), `ruleset-shadowrun`, `mush-economy` (A′, the rolling window, the obligation and contract seams, `InstrumentDisplay`, `contract_notice`, the M4 fold, DL-05, DL-08), `mush-server` (DL-04, wiring) | `sr.nuyen` + the idempotent migration; lifestyle obligations on an `sr.month` cadence; the ephemeral party subsystem; the bounded run board; run templates over E4b2's roles / outcome table / roster lock; `[shadowrun.economy]`; chargen as one live cash authority | uncontrolled faucets, board spam, an org-treasury drip, a fixer skimming, split arithmetic, party lifecycle |
| **E6b** | `mush-scenes`, `mush-canon` (DL-09's read seam), `mush-economy` (`report_stolen`, the DL-03 deposit seam, DL-11's vendor accept, DL-09, C16), `ruleset-shadowrun` | the facilitator-sealed **`scene.outcome`** producer and its registration; the four settlement routes through one condition alphabet; deterministic fence pricing with title ≠ possession; typed `report_stolen`; delayed consignment with escrow-before-custody; the augmentation record | forged seals, prose settlement, fence laundering, consignment custody races |
| **E6c** | `mush-market` (new), `ruleset-shadowrun` | catalog + content packs and the representation policy; listings, order books, quotes with declared expiry clocks; the **safe public projection**; vendors from stock / market source / procurement with engine-resolved availability tests; NPC market makers with explicit budgets | market existence oracles, stale quotes, market-maker insolvency |

**Dependencies** (no cycle; neither `mush-economy` nor `mush-org` gains an edge): `mush-party →
{core}` only — it is not a principal and must not learn about money (answers 5, 21). `mush-market →
{core, org, economy}` — listings name principals and settle through contracts; the economy never
depends on the market (doc 08 D11, doc 10 §14). `mush-scenes → {core, canon, proto}` **unchanged** —
the seal's authority is `perm::has_role` (`crates/mush-scenes/src/commands.rs:46-48`), so no
`mush-org` edge is needed. `ruleset-shadowrun` gains `{economy, org, party, faction}` in E6a
(`faction` for board membership — M5); `canon` arrives with E6b and `market` with E6c. Today it
depends only on `{mush-ruleset, mush-dice, mush-core, mush-jobs, mush-proto}`
(`crates/ruleset-shadowrun/Cargo.toml:9-19`) — every edge is new, as E5c's were.

**Module order** (`server_modules_with`, `crates/mush-server/src/modules.rs:102-233`: comsys, canon,
org, faction, scenes, sim, tactics, economy, mail, jobs, housing, weather, script): `mush-party` is a
server module and sits anywhere before the rulesets; it reads no economy state and the economy reads
none of it. The board mirror is an `EconomyObserver` frozen at wiring (`with_observers`, added when
`shadowrun` is enabled — the `issuers` pattern at `modules.rs:114-117`). `mush-market` boots
**after** `mush-economy` (it validates listings against real accounts, assets and vaults at boot —
the argument that put economy after sim); `ruleset-shadowrun` stays in `ruleset_modules` after
`server_modules` (`modules.rs:315`, built with `::new()` and no config path today).

**Economy seams each cut consumes as built.** *E6a:* `create_system_obligation` + the `[economy]`
mint cap (`crates/mush-economy/src/api.rs:1083-1129`, `config.rs:32` — 50,000 major units/period);
cadence clocks (`clock.rs:210-249`); the `@contract/*` surface with outcome-indexed terms
(`contract.rs:236-276`), `ContractRoles` (`contract.rs:528-537`), roster lock + amendment
generations, named adjudicator + `EvidenceKind` (`contract.rs:487-500`),
`EconomicVisibility::Audience` (`record.rs:340-365`), `ContractAuth` purposes, the plan-first
terminal-transition rule, and the per-class `contract_show` render (08c §5.1). *E6b:*
`with_canon_outcome_kinds` (`module.rs:635-639`, type `contract.rs:322`), the `CanonRecorded`
observer with its kind + **mandatory source** + party-intersection gates (`attest.rs:217-255`,
`canon_set_accepts` at `contract.rs:1746-1753`), `TypedCondition` (`contract.rs:399-441`), the
`DepositInto` custody lock, `After` + `stamp_observed`, and
`Encumbrance::{Disputed, Lease, Collateral}` (`record.rs:510-543`). *E6c:* `CatalogRef`
(`record.rs:556-562`, set nowhere today), `InstrumentKind::{Commodity, Entitlement}`
(`record.rs:52-61`), `issue_title` + `with_title_issuers` (`api.rs:1552`),
`may_view_account`/`mask_account`, and the `FactProvider` seam (`facts.rs:68-77`; worked example
`crates/mush-jobs/src/facts.rs:35-60`).

**Everything E6 adds to `mush-economy` — the whole list** (re-grounded at `ce43714`: 0.2's list was
stale — G1 — and `@econ/cadence` shipped in E5b, §11 q2):

1. **DL-01 A′** in `contract_amend`/`contract_offer`/`build_pay` (+ a `Draft`-with-escrow expiry in the sweep, M10) — E6a, S. The smallest change: `contract_amend` takes the author as the acting party (or the issuer); if every acceptance is the author's it skips the unwind, otherwise a partial `refund_plan_except` releases only the holds backing other parties' considerations (key `contract/{id}/refund/{gen}`, `ContractAuth::unwinding`) and `acceptances.retain(party == author)` keeps their hold ids in `escrow`; `contract_offer` re-stamps the surviving acceptances' `consent_hash` **iff** every consideration with `from == author` is claimed by a `Held` hold in `qualifying_escrow`, else refuses; `build_pay` refuses changing a consideration a `Held` hold backs; `amendments` bumps only when a refund posted. Tests: payer-amend keeps acceptance + hold; fixer-issuer amend voids the payer's; a changed own consideration is refused at offer; cancel after A′ refunds; reboot loads (boot's `Draft => {}` arm adopts nothing it should not).
2. **DL-10** `ApprovalRule.window { total, period_secs }`, journal-derived per-account rolling sum, evaluated with the threshold — E6a, S (D-E6-21; `@account/approval … window=<total>/<secs>`, doc authority on the account, E2/E3 docs unchanged). `EconomySnapshot` carries no journal, so `post_with`/`prepare_commit` hands `evaluate_approvals` a `spent: BTreeMap<AccountId, Amount>` for the plan's debit accounts through `ValidateEnv`; the sum is the account's negative lines plus `Placed` hold effects over `posted_at > now − period` (wall clock), a refund never re-opening it; a RAM-only per-account `VecDeque<(posted_at, Amount)>` maintained in `index_transaction` and rebuilt at boot from replay keeps it O(entries in window). Over the window ⇒ `NeedsApproval` on the threshold's own path. **Verify fold (M1):** an obligation period post is exempt from the *window* clause (never the threshold) through `AuthEval::window_exempt`, and still counts toward the account's window afterwards — the sweep debits, so the original "the sweep never places, so no obligation is affected" was false.
3. **`create_obligation` / `cancel_obligation` / `reinstate_obligation`** for `ObligationCause::Ruleset` — debtor never `System`, actor debit authority checked — E6a, S. No player-debit creator exists at HEAD (`@obligation/create` is staff-only and stamps `cause: Staff`; `offer_lease` is contract-shaped; `create_system_obligation` needs the faucet to own the debit account). The seam: debtor account owner never `System`, both accounts `Active` on one instrument, the actor holds debit authority on the debit account (`WorldAuth::authorize` — the player for `+lifestyle/set`, the approver at chargen), `minted_by: None`; cancel/reinstate gated on the same namespace (the `set_system_obligation` lock shape). `ObligationAuth` already permits a non-`System` debit on the record's own account.
4. **`offer_contract` / `accept_contract` / `amend_contract`** seams (the `offer_lease` shape) so a ruleset never types `@contract` — E6a, S–M. Every `@contract` builder is private and `put_contract` is `pub(crate)`; a nested `cmd::dispatch` is reachable from a handler but has no precedent in an E crate and re-runs per-invocation gates.
5. **`InstrumentDef.display: Option<InstrumentDisplay>`** + one `render_amount` — E6a, XS (E2-C16, 08g §12.1 item 4). `display` goes after `status`, `#[serde(default)]`, `{ symbol: SmolStr, position: Prefix | Suffix }`; consumers are the `to_decimal_string(scale)` sites and the five raw-minor renders of 08g §3; fold item 7 (E2-C18, `register_instrument` refuses an existing id — today an upsert) lands first.
6. `pub contract_notice` and the hiding helpers (E5b-D36/37) — E6a, XS. `notice_lines` and `show_class` are private; `may_view_contract` is `pub`; export `contract_notice(ctx, &EconomicContract) -> Vec<String>`.
7. (E6b) `AssetOp::ReportStolen` + `@asset/stolen` (08e §4 item 6, D-E6-18); (E6b) the DL-03 deposit seam; DL-11's vendor accept (`with_party_caps`) — first for E6b's NPC fence (M15), reused by E6c's vendors and makers (G8).
8. Wiring: `mush-party` in `server_modules_with`; the SR board observer in `with_observers` when `shadowrun` is enabled; `ruleset_modules(.., sr)`; `[shadowrun.economy]` in `mush-config`. Later cuts register `with_canon_outcome_kinds(vec!["scene.outcome"])` (E6b; `modules.rs:206`, empty today) and `with_title_issuers([… "sr.market"])` (E6c; `modules.rs:199`, empty today). Registration, not code.
9. **M4 fold:** `check_structure_basic` refuses a `System` party or `from` — E6a, XS; it makes D-E6-4's refusal structural.
10. **DL-08:** stale-contract paging is deadline-aware (skip while an unlapsed deadline exists) and the 7 d `STALE_CONTRACT_AGE_SECS` const becomes `[economy].stale_contract_age_secs` — E6a, XS; today every 14-day run would be paged at day 7.
11. **DL-05:** `EconomicEvent::EscalationOpened`, fanned out by the server's `broadcast_wizards` on the ladder's schedule (§3.3) — E6a, S.

**K fold (08i §7 Q16 and Q12, rulings 2026-09-04; 08i §9).** Items 3 and 4's owner-exported
`pub fn` seams — `create_obligation` and `offer_contract` / `accept_contract` / `amend_contract` —
are sanctioned delegation under 09 §C10 while the obligation and contract records stay conventional:
the seam alone validates and constructs `mush-economy`'s post-images, callers move to the
provider/bridge path once the family migrates, and a composed owner may hold an `OwnerId` with zero
typed families (answer 16). Every mutating `Registrar::command` verb E6 adds is entered in the pinned
rule-1 allowlist in `mush-contract-tests`, categorized per entry (expected `legacy-post-image` with
its future family; the economy carve-out is never automatic), in the same commit as the cut that
adds it (answer 12 — the narrower 09 §7 rule-1 exception).

---

### 2.1 Step one as built (2026-09-04)

The economy half of E6a — items 1–6 and 9–11 of the list above — on the merged main after
`0afe161` (SP1), built as ten items S1–S10 in that order, each in a `// ---- E6a step 1: <id> ----`
section with tests named `e6s1_<id>_…` (`crates/mush-economy/tests/{e4b1,e4b2,e4c}.rs`,
`src/render.rs`; `crates/mush-server/tests/{e6a_inbox,e6a_alerts}.rs`). Nothing here is
Shadowrun-shaped; the ruleset half (§3) calls the seams below and adds no economy code. All money
still moves through the journal under an authority; every persisted addition is a `TryFrom` wire with
a cap; every tick/observer/sweep path is panic-free.

**S1 — A′, consent by authorship** (`contract_live.rs`, `contract.rs`, `sweep.rs`).

- `contract_amend` runs `pub(crate) fn amend_core(ctx, c, author: &PrincipalId) ->
  Result<AmendOutcome { kept, voided, refunded }, ContractError>` with `author = amend_author(ctx, c)`
  — the party the actor stands for, else the issuer. As implemented: (a) admitted are `Offered`, or a
  live (`Active`/`ReadyToSettle`) contract with **no deliverables** all of whose acceptances are the
  author's (see deviation 2); (b) refused at `MAX_AMENDMENTS`, while `pending` is set, and when
  `!consent_intact()` (`ConsentMismatch` — `@contract/cancel` is the route); (c) acceptances
  partition by party: the author's are **kept**, with the holds backing the author's own
  considerations (`qualifying_claims` filtered on `consideration.from == author`); every other
  party's is **voided**; (d) if anything was voided, `pub fn refund_plan_except(c, snapshot, keep:
  &PrincipalId) -> Result<EconomyPlan, ContractError>` releases only the qualifying holds whose
  consideration is not the author's, under `contract/{id}/refund/{amendments}` with
  `ContractAuth::unwinding`, and `amendments` bumps **only then**; refund and `Draft` doc ride one
  batch (`post_with_companions`) — `Posted` ⇒ Draft + `ContractChanged{Draft}`, `AlreadyPosted` ⇒
  the doc alone, `NeedsApproval` ⇒ `Refused(ApprovalRequired)`, any other refusal ⇒ **nothing
  changed** (deviation 1; M16: no generation bump on a refusal or an author-only amend); (e) kept
  parties hear the contract is back in draft with their acceptance standing, voided parties that
  theirs was released; `satisfied` resets.
- `contract_offer` = `check_offer` (structure, offerability, the per-party cap, and
  `AcceptanceUnbacked` unless every surviving acceptance's party is `pub fn party_escrow_backed(c,
  snapshot, party)` — each consideration with `from == party` claimed by a `Held` hold in
  `qualifying_escrow`) then `freeze_and_offer`, which re-hashes the terms and **re-stamps every
  surviving acceptance's `consent_hash`**.
- `build_pay` refuses changing a consideration a `Held` hold backs; `@contract/cancel` releases it.
- Sweep (M10): a `Draft` with qualifying escrow past its deadline **expires** (refund) instead of
  being collected on the TTL. `expire`'s forfeit-eligible `bound` is now `is_live() &&
  fully_accepted() && parties > 1`.
- Tests: `e6s1_s1_*` ×6 (`e4b2.rs`); three E4b2 pins that assumed the whole-surface unwind were
  re-pinned (`an_amendment_refunds_every_other_partys_acceptance_and_keeps_the_authors`,
  `an_amended_contract_refunds_again_instead_of_hitting_its_first_refunds_key`,
  `vf_m3_a_refused_unwind_stays_live_and_blocks_an_amendment`).

**S2 — the rolling window** (`record.rs`, `engine.rs`, `module.rs`, `boot.rs`, `api.rs`,
`commands.rs`).

- `ApprovalRule.window: Option<ApprovalWindow>` (`#[serde(default)]`); `ApprovalWindow { total:
  Amount, period_secs: i64 }` through a wire — `total` non-negative, `period_secs` in
  `1..=MAX_APPROVAL_WINDOW_SECS` (90 d) — `ApprovalWindow::new(total, period_secs) -> Option`.
- `ValidateEnv.spent: BTreeMap<AccountId, Amount>`; `evaluate_approvals` escalates when `debit >
  threshold` **or** the rule has a window and `spent + debit` overflows or exceeds `window.total` —
  the same `NeedsApproval` path, deltas unchanged.
- The computation. `EconomyStore.debit_window: BTreeMap<AccountId, VecDeque<(posted_at, Amount)>>`
  is RAM-only and fed by **one** function, `pub(crate) fn note_window_debits(window, tx)`, from
  both `index_transaction` and boot's replay: per account, the transaction's negative lines **plus**
  its `Placed` hold effects **minus** what the same transaction `Captured` (the placement already
  counted — a settlement is not a second spend); `Released`/`Expired` subtract nothing (a refund
  never re-opens the window); entries older than `posted_at − MAX_APPROVAL_WINDOW_SECS` pop from the
  front. `pub fn spent_in_window(&self, account, now, period_secs) -> Amount` sums entries with
  `posted_at > now − period_secs`, saturating to `i64::MAX` (over any window — fail closed).
  `post_with_companions` fills `spent` through `spent_for_plan` for every account the plan debits or
  places on **whose rule carries a window**; every other account reads as zero. Wall clock, never IC.
- `@account/approval <id>=… window=<total>/<secs>|off` (`window=off` restores threshold-only);
  `@account/show` prints `rolling window: <total> per <secs> s`.
- Tests: `e6s1_s2_*` ×4 (`e4b2.rs`) — over-window escalates on the threshold path; a capture is
  not a second spend; a refund never re-opens; the tail survives a reboot.

**S3 — ruleset obligations** (`api.rs`). `pub fn create_obligation(ctx, spec: ObligationSpec) ->
Result<ObligationId, EconomyError>` (`ObligationSpec { debit_account, credit_account, amount,
schedule, grace, notify_on_success, cause }`; `cause` must be `ObligationCause::Ruleset { .. }`; the
debtor's owner never `System`; both accounts `Active` on one instrument; the actor holds debit
authority on the debit account through `WorldAuth::authorize`; `minted_by: None`). `pub fn
cancel_obligation(ctx, id, namespace: &str) -> Result<(), EconomyError>` and `pub fn
reinstate_obligation(ctx, id, namespace: &str) -> Result<bool, EconomyError>` — only the namespace
stamped in the cause may touch the record (the `set_system_obligation` lock shape); `reinstate`
returns `false`, writing nothing, when the record is not delinquent or a holder (the ladder, a
suspension) still keeps it out. Tests: `e6s1_s3_*` ×4 (`e4b1.rs`).

**S4 — the contract seams** (`contract_live.rs`), each reusing the command's core so there is one
path.

- `pub fn offer_contract(ctx, spec: ContractSpec) -> Result<ContractId, ContractError>` —
  `ContractSpec { name: SmolStr, scope: Option<OrgScopeId>, parties: Vec<PrincipalId>,
  considerations: Vec<ConsiderationSpec { account: AccountId, amount: Amount }>, outcomes:
  Vec<OutcomeTerms>, remainder_to: AccountId, conditions: Vec<TypedCondition>, adjudicator:
  Option<AttestorSet>, evidence: Vec<EvidenceKind>, deadline: Option<EconomicDeadline>, on_expiry:
  Expiry, visibility: EconomicVisibility, roles: ContractRoles, deterministic: bool }`. The issuer is
  the actor, or the organization owning `scope` when the actor acts for it (`resolve_issuer`); the
  record is built with the `@contract` builders (`build_party`, `build_pay`, …), then `check_offer`
  + `freeze_and_offer`. An outcome table and at least one consideration are required;
  `conditions` are `Attestation` only unless `deterministic` (E6b's routes).
- `pub fn accept_contract(ctx, id: ContractId) -> Result<AcceptOutcome, ContractError>`;
  `pub enum AcceptOutcome { Accepted { escrow: Vec<HoldId>, activated: bool },
  NeedsApproval(ApprovalRequestId) }` — `accept_core`, the command's core.
- `pub fn amend_contract(ctx, id, amend: ContractAmendment) -> Result<(), ContractError>`;
  `#[derive(Default)] ContractAmendment { add_parties: Vec<PrincipalId>, outcomes:
  Option<Vec<OutcomeTerms>>, on_expiry: Option<Expiry>, reoffer: bool }` — `amend_core` under A′,
  the edits, then `check_offer` + `freeze_and_offer` when `reoffer`.
- Each refuses a frozen economy (`Infeasible`) and answers every standing failure with
  `ContractError::NoStanding` (renders as "No such contract."). Tests: `e6s1_s4_*` ×5 (`e4b2.rs`).

**S5 — `InstrumentDisplay` + one `render_amount`** (`record.rs`, `render.rs`, `commands.rs`).
`InstrumentDef.display: Option<InstrumentDisplay>` (`#[serde(default)]`); `InstrumentDisplay {
symbol: SmolStr, position: SymbolPosition::{Prefix, Suffix} }` through a wire (`1..=MAX_SYMBOL_BYTES
= 8` bytes, no whitespace or control chars). `pub fn render_amount(snapshot: &EconomySnapshot,
instrument: &InstrumentId, amount: Amount) -> String` and `pub fn render_amount_on(snapshot,
account: AccountId, amount)` — scale-aware, the prefix after the sign (`-¥5`, `12.50€`), an unknown
instrument as bare minor. `commands::money(ctx, &InstrumentId, Amount)` / `money_on(ctx, AccountId,
Amount)` wrap it for handlers; every amount render in `commands.rs`, `contract_live.rs` (the pay
reply, `contract_show`, shares, the obligation deliverable line) and the sweep's mint-cap detail goes
through it. `@instrument/register <code>=<scale> [display=<symbol>:prefix|suffix]`. Tests:
`e6s1_s5_*` — one unit test in `render.rs`, two in `e4b1.rs`, one in `e4b2.rs`.

**S6 — `System` as a party is structural** (`contract.rs`): `check_structure_basic` refuses a
`System` party or a `System`-owned consideration `from` with `ContractError::SystemParty` (renders
"No such account."); `build_party` refuses `System` ("No such principal."), `build_pay` a
`System`-owned account under hiding. Tests: `e6s1_s6_*` ×2 (`e4b2.rs`).

**S7 — DL-08** (`config.rs`, `mush-config`, `mush-server/src/config_ext.rs`, `sweep.rs`):
`[economy].stale_contract_age_secs` (default 7 d, the old const; `≤ 0` disables paging), and
`page_stale_contracts` skips a contract whose own deadline has not lapsed. Tests: `e6s1_s7_*` ×3
(`e4c.rs`).

**S8 — the notice and hiding helpers**: `pub fn contract_notice(ctx, &EconomicContract) ->
Vec<String>` (= `notice_lines` — never an amount or an account id), `pub fn refuse_contract(ctx, c,
&ContractError) -> CmdError`, `pub fn refuse_contract_offer(ctx, &ContractError, named: impl
IntoIterator<Item = NamedResource>) -> CmdError`; `parse_opts`, `parse_account`, `parse_amount`,
`parse_principal`, `refuse`, `refuse_named`, `is_hidden`, `mask_account`, `render_principal`,
`money`, `money_on` are `pub`. Test: `e6s1_s8_*` ×1 (`e4b2.rs`).

**S9 — DL-04 `OrgInbox`** (`notice.rs`; `mush-server/src/econ_inbox.rs`).

- `pub fn deliver_notice(ctx, who: ObjId, msg)` — connected ⇒ `sys_to_actor(who, "economy", …)`,
  else `pub fn queue_notice`, which dedupes identical text, keeps the newest `MAX_PENDING_NOTICES =
  32`, and persists the **leaf doc `econ:notice:{obj}` = `Vec<PendingNotice { at: i64, text:
  String }>`**, each `text ≤ MAX_NOTICE_BYTES = 240` (wire-checked; at boot an unreadable or
  oversized entry is dropped and the doc capped at the 32 newest — a leaf, never a boot failure).
  `ReconnectNotice` on `HookPoint::Connected` renders `ECONOMY: n notice(s) while you were away.` +
  the lines and deletes the doc.
- `pub fn principal_holders(w, &PrincipalId) -> Vec<ObjId>`: an object is its own holder; `System`
  has none; an organization's are its members and grantees who hold `economy.account.post` on its
  root scope **by grant** (staff are not enumerated), in id order. `pub fn acts_for(w, actor, who)`
  is the matching predicate. `OrgInbox: EconomyObserver` turns `ContractChanged`
  (Offered/Active/Settled/Disputed/Cancelled/Expired; Draft and ReadyToSettle stay silent) and
  `ObligationChanged` on an org principal into one line per holder. Wired in `server_modules_with`
  beside the canon bridge and housing.
- Tests: `e6s1_s9_*` ×3 (`mush-server/tests/e6a_inbox.rs`).

**S10 — DL-05** (`record.rs`, `sweep.rs`, `observer.rs`; `mush-server/src/econ_alerts.rs`,
`world_task.rs`): `EconomicEvent::EscalationOpened { id: EscalationId, subject: EscalationSubject,
level: u8 }`, emitted by `announce_escalation` at every step of the ladder's own schedule — a fresh
page, a reopen after an acknowledgement whose cause came back, and each `renotify` reminder (1 h,
then daily). `StaffAlerts: EconomyObserver` (RAM-only `Mutex<BTreeMap<EscalationId, (reopened,
notifications)>>`) dedupes per record step and writes `ECONOMY ALERT (opened|reopened|still open):
<record.render()>` plus the first recommended command to every connected wizard
(`world_task::connected_wizards`, the `broadcast_wizards` audience); `page_dropped` covers the
event. Tests: `e6s1_s10_*` ×3 (`mush-server/tests/e6a_alerts.rs`).

**Verify fold (2026-09-04).** An adversarial verify of step one over three lenses — D disclosure
and bounds, R durability and replay, M money and consent — confirmed twenty-eight findings; all
twenty-eight are folded in, each with a regression test in a `// ---- E6a step 1 verify fold ----`
section named `vf_<lens><n>_…`.

- **D1** `may_view_obligation` tests the parties with `WorldAuth::acts_for`, not identity, so an
  org's own officer can read the record the inbox writes to them about; and `OrgInbox` filters every
  holder through the record's own view gate, so a line can never outrun it again (`commands.rs`,
  `econ_inbox.rs`; `vf_d1_…`, `e6a_inbox.rs`).
- **D2** `render_amount_on` takes the caller's `may_view_account` verdict; an unviewable account and
  an unknown one render identically. `money_on` asks the gate once, for every call site
  (`render.rs`, `commands.rs`; `vf_d2_…`, `e4b2.rs`).
- **D3** `principal_holders` is bounded by `MAX_NOTICE_HOLDERS` (32, id order, logged), so one event
  is at most 32 durable notice writes (`notice.rs`; `vf_d3_…`, `e6a_inbox.rs`).
- **D4** the rolling-window index tracks **only** accounts whose rule carries a window; a rule that
  gains one is rebuilt from the journal tail and one that loses it is dropped; the per-account
  `MAX_WINDOW_ENTRIES` (4 096) latches `WindowTail::saturated`, which reads as over every window
  (fail closed); pruned on read from the sweep's GC as well as on write (`module.rs`, `api.rs`,
  `boot.rs`, `sweep.rs`; `vf_d4_…` ×2, `e4b2.rs` + `module.rs`).
- **D5** `StaffAlerts.paged` is swept against the live ladder on every economic event and capped at
  `MAX_PAGED_RECORDS` (4 096) (`econ_alerts.rs`; `vf_d5_…`, `e6a_alerts.rs`).
- **D6** `PendingNotice` carries `about: Option<PrincipalId>` and `render_pending` re-asks `acts_for`
  at delivery, so a queued line is re-gated against live standing (`notice.rs`, `econ_inbox.rs`;
  `vf_d6_…`, `e6a_inbox.rs`).
- **D7** `render_principal` renders a **hidden** organization as “an organization” to anyone who is
  not staff, a member or a grantee (G3 brought forward) (`commands.rs`; `vf_d7_…`, `e6a_inbox.rs`).
- **D8** `@account`'s help documents `window=` and says what a budget is, beside the threshold that
  is not one (`commands.rs`; `vf_d8_…`, `e4b2.rs`).
- **D9** the inbox distinguishes a **created** obligation from a resumed one (`econ_inbox.rs`;
  `vf_d9_…`, `e6a_inbox.rs`).
- **D10** boot trims a notice document's raw array to `MAX_PENDING_NOTICES` **before** parsing its
  entries (`boot.rs`; `vf_d10_…`, `e4b2.rs`).
- **R1** every fallible step of `amend_contract` runs **before** `amend_core` posts — the amendment
  is projected by a new pure `amended_draft` + `apply_amendment` and checked whole — and the
  re-offer is non-fatal: `AmendReoffer::{Amended, NotReoffered}` replaces the `()` return. Belt:
  `install_commit` calls `Outbox::pin_deferred`, so a ruleset handler's later `Err` can never discard
  a landed post's batch. `offer_contract`, `accept_contract`, `create_obligation` and
  `cancel`/`reinstate_obligation` were audited for the same shape and are clean (`contract_live.rs`,
  `api.rs`; `vf_r1_…` ×2, `e4b2.rs`).
- **R2** the amendment generation moves when a refund posts **or when one already has** (a listed
  escrow hold for a voided party's consideration that the journal says is terminal), so a lost
  companion write cannot leave the record on a spent key (`contract_live.rs`; `vf_r2_…`, `e4b2.rs`).
- **R3** nothing is pruned at index time: age is a question about *now*, and a post's own clock is
  not now. The tail is bounded by length on write and by age on read, against the world's clock and
  never below the tail's own high-water mark (`module.rs`; `vf_r3_…`, `e4b2.rs`).
- **R4** `StaffAlerts` records a step only when the audience was non-empty, and a Wizard-or-better's
  connect re-announces the ladder's open backlog (`MAX_ANNOUNCE_BACKLOG` 32) (`econ_alerts.rs`,
  `sweep.rs`, `notice.rs`; `vf_r4_…`, `e6a_alerts.rs`).
- **R5** `on_world_loaded` announces boot's own opened set (`MAX_BOOT_ESCALATIONS`), which R4's
  connect-time delivery then carries to the first staffer (`module.rs`; `vf_r5_…`, `e6a_alerts.rs`).
- **R6** the sweep delivers every standing notice queue whose owner is connected
  (`MAX_NOTICE_DELIVERIES_PER_SWEEP` 32), so a session held across a restart — `Engine::reattach`
  fires no `Connected` — still reads it (`notice.rs`, `sweep.rs`; `vf_r6_…`, `e6a_inbox.rs`).
- **R7** `collect_draft` and the sweep's selection share one predicate, `qualifying_escrow`
  (`sweep.rs`; `vf_r7_…`, `e4b2.rs`).
- **R8** a dropped announcement gets its own `EscalationSubject::DroppedPage`, so it can no longer
  suppress the level-3 freeze record (`escalation.rs`, `observer.rs`, `commands.rs`; `vf_r8_…`,
  `e4b1.rs`).
- **R9** `notice.rs` carries no `expect`/`unwrap`/`panic!` on the queue path (`notice.rs`;
  `vf_r9_…`, a source scan in `e4b2.rs`).
- **R10** `declared::reconcile` re-stamps schema 1 on a cadence whose name is no longer declared —
  the clock itself untouched, the change logged (`declared.rs`; `vf_r10_…`, `e4b1.rs`).
- **M1** the obligation-period exemption from the rolling window; see D-E6-21 as amended
  (`engine.rs`, `sweep.rs`; `vf_m1_…`, `e4b2.rs`).
- **M2** an escrowed `Draft` with no deadline expires on `DRAFT_TTL_SECS`, and `page_stale_contracts`
  now admits an escrowed draft with its own wording and its own commands (`sweep.rs`; `vf_m2_…`,
  `e4b2.rs`).
- **M3** `ruleset_obligation` asks `WorldAuth::authorize(actor, debit_account, −amount)` — the
  namespace says *which records*, never *who is asking*. `reinstate_obligation` re-anchors the
  schedule and so forgives arrears (D-E6-23: the ruleset enforces “pay the arrears first”), and the
  seam refuses a terminal record; both are documented in `api.rs` (`api.rs`; `vf_m3_…`, `e4b1.rs`).
- **M4** the amended draft keeps **every** escrow-purpose `Held` hold the refund did not release
  (deduplicated), so a hold claiming no consideration is no longer orphaned. The live seams cannot
  produce that shape — `build_pay` guards it and a patched hold doc freezes the economy at boot as
  drift — so the regression reaches it through an edited-and-rehashed stored surface
  (`contract_live.rs`; `vf_m4_…`, `e4b2.rs`).
- **M5** `freeze_and_offer` activates a re-offer that is already `fully_accepted()`
  (`contract_live.rs`; `vf_m5_…`, `e4b2.rs`).
- **M6** boot's quarantine page for a `SystemParty` structure failure recommends `=refund` only
  (`boot.rs`; `vf_m6_…`, `e4b2.rs`).
- **M7** `amend_author` prefers the **issuer** when the actor acts for it, before the generic
  `acting_party` fallback (`contract_live.rs`; `vf_m7_…`, `e4b2.rs`).
- **M8** an unviewable or unresolvable account renders in the *record's* instrument — the contract's
  or the obligation's — and bare minor only when the instrument itself is unknown (`render.rs`,
  `commands.rs`, `contract_live.rs`, `sweep.rs`; `vf_m8_…`, `e4b2.rs`).

**Deviations from the list above, with reasons.**

1. A refused partial refund (an approval rule on the escrow account, a frozen hold) **refuses the
   amend outright and changes nothing** — no pending unwind is recorded. An amend is not a terminal
   transition and its refund is not owed; `@contract/cancel` stays the retrying, paging route
   (`vf_m3` was re-pinned to exactly that flow).
2. An author-only amend is admitted on a **live** contract with no deliverables. The last acceptance
   activates, so a one-party or author-only contract is `Active` the moment the author accepts;
   `Offered` alone would have made `amend_contract` useless for the fixer-issued run it exists for.
   Any other party's acceptance or any deliverable still needs the unwind; `expire`'s forfeit needs
   more than one party for the same reason.
3. Item 5 named five raw-minor render sites; housing's (`Property.upkeep`) was removed by E5b-D34 in
   the look-back fold, so four were converted, and `render_amount_on` is the pub helper E6's own
   renders use.
4. The K fold's allowlist rule: step one adds **no primary verb** — only the switches `window=` and
   `display=` — and the tripwire pins `CommandSpec::name` only, so `legacy-handlers.toml` is
   unchanged (its test was run).
5. Item 11 named `broadcast_wizards`; the fan-out is an observer (`StaffAlerts`) on the same
   `connected_wizards` audience, so the economy still emits ids only (D-E4-13) and the server alone
   decides who hears it.

**What E6a's ruleset half now calls** (§3): `create_obligation` with `ObligationCause::Ruleset`
on the `sr.month` cadence for lifestyle, `cancel_obligation`/`reinstate_obligation` on
`+lifestyle/set`; `offer_contract`/`accept_contract`/`amend_contract` for the run board (a fixer's
amend keeps the fixer's acceptance and refunds the runners'); `render_amount`/`money_on` with
`display = ¥:prefix` for `sr.nuyen`; `contract_notice` and `deliver_notice` for board lines; the
window on org treasuries through `@account/approval … window=`; `EscalationOpened` reaches staff
with no Shadowrun code at all.

---

## 3. E6a — run and pay

### 3.1 Records

```rust
// ---- mush-party (new; game-neutral, NOT an economic principal) -------------
pub struct Party {
    pub id: PartyId,  pub name: SmolStr,  pub leader: ObjId,
    pub members: Vec<ObjId>,               // accepted, ≤ MAX_PARTY_SIZE (8); order is display only
    pub invited: Vec<Invitation>,          // { who, by, at, expires_at }, ≤ MAX_PARTY_INVITES (16)
    pub roles: BTreeMap<ObjId, SmolStr>,   // "face", "decker" — labels, never authority
    pub status: PartyStatus,               // Forming | Locked { by: ContractId } | Disbanded { how }
    pub expires_at: i64,                   // idle expiry, refreshed by any membership change
    pub revision: u64,
}
// ---- ruleset-shadowrun -----------------------------------------------------
pub struct SrProfile {                     // `sr:profile:{obj}`
    pub account: Option<AccountId>,        // opened at chargen approval or migration
    pub lifestyle: LifestyleTier,          // street|squatter|low|middle|high|luxury (data.rs:225-232)
    pub upkeep: Option<ObligationId>,      // the sr.month obligation; None when the tier costs 0.
                                           // A CACHE: the record self-identifies by cause
                                           // Ruleset{"sr.lifestyle"} + credit = system:sr.lifestyle (DL-07)
    pub unpaid_periods: u8,                // keyed on the sr.month ordinal of the last miss, never
                                           // wall time; carried across a replacement (M11)
    pub arrears: Amount,                   // what +lifestyle/pay posts; keyed on the same ordinal
    pub minted: Option<Mint>,              // { key, transaction, at } — the chargen allotment (M12, M13)
    pub migrated: Option<Migration>,       // { from_sheet_value, approved_at, transaction, at } —
                                           // SheetDoc has no revision (mush-ruleset/src/doc.rs:60-70)
    pub revision: u64,
}
pub struct RunTemplate {                   // `sr:run:{id}` — the BOARD entry; terms live in the contract
    pub id: RunId,
    pub contract: ContractId,              // money, authority and consent are E4b2's, never duplicated
    pub principal: PrincipalId,            // accountable payer: Object | Organization — never System (D-E6-4)
    pub notice: RunNotice,                 // { name, window, kinds, johnson: Option<PrincipalId> } —
                                           // copied at post, rendered by +run/show (M5); `johnson` is
                                           // the visible representative when announced (answer 4, G7)
    pub board: BoardVisibility,            // Public | Faction(FactionId) | Invitation { party: PartyId }
                                           // — enforced ruleset-side, never the contract's Audience (M5)
    pub faction: Option<FactionId>,        // the Faction(_) payload, denormalized for +run/list faction
    pub slots: u8,  pub posted_at: i64,
    pub deadline: EconomicDeadline,        // the contract's own (RealUtc): nothing else expires an
                                           // Offered contract holding escrow (M8)
    pub expires_at: i64,                   // = that deadline's instant, ≤ BOARD_TTL
    pub party: Option<PartyId>,  pub revision: u64,
}
```

`RunTerms` is built at `+run/post` and written straight into E4b2's `ConsentSurface` through the
`offer_contract` seam (§2 item 4): one `Consideration` from the payer's account; `ContractRoles` plus
`ConsentSurface.adjudicator` and `evidence`; and an **outcome table** whose `payouts` name each
frozen recipient account exactly (`contract.rs:236-256`). Answer 7's five slots — runner payout,
fixer fee, expense advance, reimbursable reserve, bonuses/penalties — are **named payout shares
inside each outcome row**, not new economy fields; the reserve is a share back to the payer's own
account, named explicitly in the failure row, because `ExpenseTreatment::Returned` is declarative
and read by nothing (`contract.rs:206-225`). `PayoutShare.to` must exist and be `Active` at offer
(`check_offerable`, `contract.rs:1604-1608`), so at post the crew's share sits provisionally on the
payer's own account and `+run/crew accept` rewrites it to the members' `SrProfile.account`s (M6).

### 3.2 State machines

```text
Party:   Forming ──payer accepts the party's application──▶ Locked{contract}
         Forming ──leader disbands / last member leaves / idle past expires_at──▶ Disbanded{how}
         Locked  ──contract Settled | Cancelled | Expired──▶ Disbanded{Settled|Released}
         Locked  ──substitution──▶ Forming   (the contract must be AMENDED first; §1.1)
Board:   Draft ──+run/post: the payer drafts as issuer, offers, and accepts (escrow) in one seam call──▶ Listed
         Listed ──+run/crew accept: an A′ amendment adds the crew as parties and rewrites every
                  outcome row to their accounts, then re-offers; the payer's acceptance and escrow stand──▶ Crewing
         Crewing ──every runner accepts (the first acceptance locks the roster)──▶ Crewed (Active)
         Listed | Crewing ──the deadline, or the payer withdraws──▶ Closed  (unbound: escrow refunded; entry dropped)
         Crewed ──the contract settles, or its deadline lapses and DISPUTES (M7)──▶ Closed
Life:    Paid ──miss──▶ Arrears{1} = Grace                (one private notice, NO downgrade)
         Arrears{1} ──miss──▶ Delinquent: one tier down; the record is REPLACED at the new tier
                     (create_obligation), unpaid_periods carried                     (answer 3, M11)
         Arrears{_} ──the next period pays (Grace) / +lifestyle/pay: arrears, then reinstate──▶ Paid  (silent)
         any ── ForgiveCause::{Downtime,TimeJump,Suspension} ──▶ unchanged, no downgrade
         Grace | Delinquent ── +lifestyle/set ──▶ REFUSED (pay the arrears first)
```

**No listing exists before its escrow does** (answer 24). `+run/post` drafts with the payer as issuer
(D-E6-19), offers and accepts through the `offer_contract` + `accept_contract` seams — no `@contract`
command is dispatched — and E4b2 places a party's consideration hold at *their* accept, exactly the
Johnson-escrow shape the shipped rig proves (`crates/mush-economy/tests/e4b2.rs:249-251`). A refused
accept (no funds, an `ApprovalRule` threshold or rolling `window`, a suspended scope) lists nothing
**and cancels the draft**, so no unfunded `Offered` contract counts against
`MAX_OPEN_CONTRACTS_PER_ISSUER` (M17). Between the A′ amendment and the re-offer a `Draft` can hold
live escrow (`/offer` refuses a closed payout account and `collect_draft` skips a draft with holds):
`+run/crew accept` retries or cancels, and the sweep expires a `Draft` whose deadline lapsed (M10).
**One downgrade per unpaid period, ever:** the downgrade is driven by `ObligationChanged` /
`missed_total` deltas, not by the arrears counter's absolute value, and a forgiven period is never a
miss — a ten-period downtime backlog forgives with a classified cause and downgrades nothing
(answer 3; doc 08 §2 forbids charging a backlog). Two E4 facts shape this (M11, G12): `Delinquent`
leaves the due pool and autopay stops (`obligation.rs:171-173`, `sweep.rs:459-461`), so the second
miss *replaces* the record one tier down; and `plan_sweep` forgives a backlog's excess but still
attempts `maximum_catch_up` in one fire, so the lifestyle schedule sets it to **1** — one fire, one
attempt, never a second miss from downtime — while the `sr.month` clock's own `catch_up` (3) forgives
on the clock first. `unpaid_periods` and `arrears` are keyed on the `sr.month` ordinal at the miss,
never wall time (E5c T3).

### 3.3 Money and faucets

`ruleset-shadowrun` registers at boot: `sr.nuyen` (`InstrumentKind::Currency`, **scale 0** —
answer 1; already the crate's documented example at `crates/mush-economy/src/ids.rs:8,85`; with
`display: Some(InstrumentDisplay { symbol: "¥", position: Prefix })` once fold item 7 (E2-C18) makes
`register_instrument` refuse an existing id — 08g §12.1 item 4), the faucets `system:sr.chargen`
(answer 17) and `system:sr.migration` (answer 18), and the sink `system:sr.lifestyle`. `sr.month` is
created by `@sr/clock` over `mush_economy::create_cadence` (`api.rs:1365-1425`), default
`CadenceSource::WorldIc` (answer 3: an explicit campaign clock, never unrestricted real time and
never a literal calendar); it is a legacy `EconomicClock::Cadence(id)` until D-LBK-7's convergence
(§9). Who may fund (answer 4): a player from their own account; an org officer holding
`contract.draft`, `contract.accept` and `economy.account.post` on the named scope, subject to that
account's `ApprovalRule` — threshold and rolling `window` (D-E6-21); staff or a player ST for a
principal they act for; a campaign through a **funded NPC organization** — never a `System`
principal directly (D-E6-4: the tree makes one unfundable, E6a makes it unofferable).

The run's issuer is its payer (M1). A `System` account named as the payer's is refused at `+run/post`
with the account's own hiding text (M4). Outcome conditions are attestations; the adjudicator
defaults to the payer (M2). A crewed run's deadline disputes (M7). The allotment and the migration
are one-shot attributed posts bounded by the priority table, not by `[economy]`'s standing-mint cap
(M12).

**Roles (answer 22, amended by the owner at 0.3).** Distinctness is enforced where a conflict of
interest exists — the adjudicator is never a party (payer, runner, representative, facilitator) and
never the poster; the representative and the facilitator are never the payer; the author and the
payer **may** coincide (a Johnson posting their own run); an organization's author is the posting
officer. Operationally (M1): `+run/post` is run by the payer — a character from their own account, or
an officer holding `contract.draft`, `contract.accept` and `economy.account.post` on the org's scope
(`scope=`), which makes the org the issuer; a fixer is `roles.author`/`representative` with a fee row
and never the issuer. Why: `@contract/amend` is issuer-only and A′ keeps only the *amending party's*
acceptance, so a fixer-issuer would refund the payer's escrow at every `+run/crew accept` and leave a
`Listed` entry standing on nothing (D-E6-19).

**Who is told (G5 — E6a prerequisites, not fix-during extras).** Contract notices reach `Object`
parties only (`contract_live.rs:2643-2663`, `sweep.rs:956-960`), and org payers are E6a's design
case: without these an org is told nothing when its run is crewed, settled, disputed or refunded.
**DL-04:** an `OrgInbox` economy observer delivers a kinds-only line to every connected holder of
`acts_for` on the org, with an offline notice per the housing pattern (server, S). **DL-05:**
`EconomicEvent::EscalationOpened` plus a `broadcast_wizards` fan-out following the ladder's schedule
(economy + server, S).

### 3.4 Commands

```text
+party  ·  /create <name>  /invite <who>  /uninvite <who>  /accept <id>  /decline <id>
        ·  /leave  /kick <who>  /role <who>=<label>  /disband
+run/list [public|faction|mine]        the board, newest first, ≤ BOARD_PAGE
+run/show <id>                         the board's own notice under the board's visibility rule (M5)
+run/post <name>=<pay> outcomes=<a,b,c> [scope=<org>] [johnson=] [adjudicator=<attestor>]
                                       [slots=<n>] [board=public|faction:<f>|invite:<party>] [deadline=]
                                       (the payer is the issuer — M1; evidence is `attest`, forced — M2)
+run/split <id>=<slot|who> <exact|N‰>  edit a slot share while still Listed — an A′ amendment (M16)
+run/apply <id>=<party>                the leader offers their party (every member needs an sr.nuyen account — M6)
+run/crew <id>=accept|decline          the payer's answer; accept = the A′ amendment (crew added, payouts rewritten, re-offered)
+run/withdraw <id>                     the payer cancels a Listed run (refund via @contract/cancel)
+lifestyle  ·  /set <tier> (next period, never the current one; refused in arrears)  ·  /pay
+nuyen                                 the derived balance post-approval; the sheet field before
@sr/economy [show]   @sr/clock <name>=<source> <period>   @sr/migrate [dry|commit] [<who>]
@sr/profile <who>                                                              (all staff)
```

Every refusal renders through the economy's existence-hiding discipline: a run, party or profile the
actor may not view is "No such run." / "No such party." — identical for nonexistent and unviewable
(08a decision 13). `+run/show <id>` renders the board's own notice (name, state, window, condition
kinds, the Johnson when announced) under the board's visibility rule — faction membership through
`mush_faction`, never the contract's `Audience` (M5). A faction is not a principal: the audience
clause of `may_view_contract` is `acts_for`, which for an org means spenders, not members, and
`MAX_AUDIENCE` is 32. So `Public` = any profiled character; `Faction(f)` = `mush_faction`
`is_member`/`membership_visible`; `Invitation { party }` = the party's members and invitees; the
contract's own visibility is `Parties` until crewed and stays so — the crew are parties, so
`Audience { members }` is unnecessary and `announce_issuer` maps to the notice's `johnson` (G7).

**Refusals at post and crew** (M2, M6, M9, M16–M18, §11 q7). `+run/post` refuses an `adjudicator=`
naming the payer or the poster; any `before`/`after`/`fact` outcome row — E6a settles by attestation
(D-E6-20), `adjudicator=` defaulting to `party:<payer>`; a payer account the poster may not view; and
a `System` account, with that account's hiding text and no draft left behind (M4). On a refused
payer accept it cancels the draft (M17). `remainder_to` is inert under a table but must still
resolve to a live account in the instrument: it is the payer's (M18). `+run/apply` refuses a member
with no `sr.nuyen` account; `+run/crew accept` derives every `PayoutShare.to` from the roster
member's `SrProfile.account`, never from input (M6), refuses a party containing the adjudicator (M9),
and stores the account-distinctness verdict from the core/org oracle at the lock (doc 10 §15 R3,
G10). `+run/split` while Listed is an A′ amendment (`editable` refuses once anyone accepted); the
generation bumps only when a refund posted, so splits never exhaust `MAX_AMENDMENTS` (M16).

`+lifestyle/set` is refused while the current record is in `Grace`/`Delinquent`; `+lifestyle/pay` is
an attributed post of the arrears to `system:sr.lifestyle` under the player's own authority, then a
reinstate. The obligation itself is created through `create_obligation` (§2 item 3) — no
player-debit creator exists at HEAD — with the player's own debit authority at `+lifestyle/set` and
the approver's at chargen.

### 3.5 Chargen and migration

**Chargen (answer 17).** Until E6c's catalog exists there is nothing to shop for, so E6a ships the
**cash authority** and keeps today's behaviour otherwise: the approval handler
(`crates/ruleset-shadowrun/src/chargen_cmds.rs:450-477`, registered `lib.rs:262-270`, which today
writes only `approved`) gains a step that, atomically after its validation, opens the `sr.nuyen`
account and mints the priority allotment (`data.rs:1177`:
`[450_000, 275_000, 140_000, 50_000, 6_000]`) from `system:sr.chargen` — a **one-shot attributed
post** like WoD's (`wod/chargen.rs:149-222`, 08d dev. 10) with the approver as actor (nobody acts
for `System`) and the deterministic key `sr.chargen/{obj}`, recorded in `SrProfile.minted`. The
`[economy]` mint cap binds `create_system_obligation` only: 450,000¥ exceeds the 50,000 major cap,
so `MintCapExceeded` is unreachable at chargen and the bound is the priority table
(`resource_nuyen[A]`) plus E5c-M5's boot warning (M12). A refused mint never fails the approval
(08d deviation 11): the character is approved with an empty account and `@sr/migrate commit`
adopts them (M13). **The 5,000¥ carry-over cap is not applied yet** — answer 17: "until shopping
exists, keep current behavior and do not apply the cap." After approval `@sheet/set …/nuyen` is
refused (the field is the chargen budget and the migration source and nothing else) — the check
lives in `sheet_set`'s map arm (`sheet_cmds.rs:118-200`), keyed on `doc.approved`; `chargen_settable`
(`chargen_cmds.rs:542-560`, `+stat`'s in-chargen denylist) is untouched — `nuyen` must stay settable
in chargen, it is the Resources budget. `+sheet` and `+nuyen` render the account balance instead of
the stored number, viewer-aware (self + staff, `may_view_account`; omitted, never placeholdered —
the `wod/commands.rs:456` pattern, G2).

**Migration (answer 18).** `@sr/migrate dry` lists every approved character with a non-zero sheet
`nuyen` and the exact mint it would make. `commit` posts once per character under the deterministic
key `sr.migration/{obj}`, binding `from_sheet_value` and the approval stamp (`Approved { by, at }`,
`chargen_cmds.rs:449-477`); `SheetDoc` carries no revision (`mush-ruleset/src/doc.rs:60-70`). It
mints from `system:sr.migration` (**not** the chargen faucet) as the same one-shot attributed post,
writes `SrProfile.migrated`, and stops treating the sheet number as authoritative. `@sr/migrate
commit` also adopts an approved character whose profile records no chargen mint (M13). The verb is
E5c-D5's migrate shape (`wod/migrate.rs:109-131`: staff, one-shot, idempotent, reports what it did)
with `dry|commit` added because this one mints. A re-run is an idempotency HIT: it re-reports the
first mint and mints nothing. A sheet edited afterwards keeps the bound value and stamp and is
reported as a mismatch, never re-minted. Boot follows E5c-D6: a lifestyle obligation self-identifies
by `cause: Ruleset{"sr.lifestyle"}` plus credit = `system:sr.lifestyle`'s account, `SrProfile.upkeep`
is a cache, `sr::boot::run` adopts or cancels, and the identity rule (`obj == key`, loud skip) is
DL-07.

### 3.6 E6a tests (~40 planned; **118 built** — 51 `mush-party`, 41 `ruleset-shadowrun/tests/e6a.rs`, 4 economy seam tests, plus the in-crate wire/config/store units; §3.7)

**Party:** create/invite/accept/decline/leave/kick/disband; size and invite caps; idle expiry; `Locked` refuses membership change; a substitution needs the amendment and returns `Forming`; reboot at every status.
**Board:** a post with no funds lists nothing and cancels the draft (M17); a listed run's escrow is real (assert `held_amount`); `MAX_OPEN_RUNS_PER_PRINCIPAL` and `BOARD_CAP` refuse the next post; an expired unbound listing closes and refunds; a faction-visible run is invisible to a non-member and renders a notice — never terms — to a faction member through `mush_faction` (M5); a non-viewer gets "No such run."; window budget: the fifth ≤ T run from an org account needs approval (DL-10); the org payer is notified through DL-04 when its run is crewed and settled.
**Money:** the split sums exactly and **no remainder reaches the fixer** (the outcome table makes `remainder_to` inert, `contract.rs:268-270`; it resolves to the payer's account — M18); a fixer fee is a named share that cannot widen after the lock; each outcome row pays exactly what it names; an advance comes back in the failure row through a share named to the payer's account; payout accounts are the members' own and a member without an account cannot crew (M6); payer-amend keeps escrow (A′); a fixer-issuer is refused at post (M1); an adjudicator naming the payer, or a party containing the adjudicator, is refused (M9).
**Faucets:** a `System` payer is refused at post with the account's hiding text and no contract was drafted (M4, D-E6-4); a campaign org funded by `create_system_obligation` posts; an over-cap `create_system_obligation` for that org is `MintCapExceeded` (the chargen allotment is not under the cap — M12).
**Settlement:** a `before`/`after`/`fact` outcome row is refused at post (M2); the payer's attestation settles the row; a deadline on a crewed run disputes, never refunds (M7); a `Draft` holding escrow past its deadline is expired by the sweep (M10).
**Lifestyle:** a paid period is silent; one miss notices (`Grace`) and does not downgrade; a second downgrades exactly one tier by replacing the record with `unpaid_periods` carried; `+lifestyle/set` in arrears is refused; a 10-period backlog with catch-up 1 downgrades nothing; ten forgiven downtime periods downgrade nothing; a tier change applies next period; a zero-cost tier creates no obligation; boot adopts a self-identifying record and cancels an orphan (DL-07).
**Chargen/migration:** approval opens the account and mints once, a second approval is a HIT, `@sheet/set …/nuyen` is refused afterwards while `+stat nuyen=` in chargen still works; `dry` writes nothing, `commit` mints once, a re-run returns the original result, an edited sheet is reported and not re-minted; a character approved with no mint is adopted by `commit` (M13).
Extend the shipped **Shadowrun run rig** (`crates/mush-economy/tests/e4b2.rs:167-213`, whose instrument is already `@instrument/register nuyen=0` at `:186`) with an `sr.nuyen`-registered ruleset world.

**Hazards on this axis.** *Board spam:* per-principal caps, escrow-at-publication, `BOARD_TTL` (= the contract deadline, M8), a page cap, and the rolling `window` on an org account against a ≤-threshold drip (M3, D-E6-21). *A fixer skimming:* every share is named and each row must sum — E4b2 already refuses an unmatched credit (`ContractPurpose::Settle` carries netted `(account, amount)` pairs, 08c §5.1); the fixer is never the issuer (M1) and payout accounts are derived, never input (M6). *An uncontrolled faucet:* D-E6-4 (a `System` payer refused at post, no draft left) plus the live mint cap on the campaign org's standing drip, re-read at every settlement; the chargen allotment is bounded by the priority table (M12).

### 3.7 As built (2026-09-05)

The ruleset half of E6a, on top of step one's seams. Nothing Shadowrun-shaped entered
`mush-economy` or `mush-party`; the one economy addition is game-neutral and named below. All money
still moves through E4 under an authority; every persisted addition is a `TryFrom` wire with caps;
every tick/observer/boot path is exhaustive and panic-free.

**`mush-party` (new crate; `mush-party → {core, serde, smol_str, tracing}`).** `Party`,
`Invitation`, `PartyStatus::{Forming, Locked{by: ContractRef}, Disbanded{how}}` and
`DisbandCause::{Disbanded, Empty, Expired, Settled, Released}` (`record.rs:95-206`), each loaded
through a wire that re-runs every constructor invariant (name ≤ 48 B, `MAX_PARTY_SIZE` 8,
`MAX_PARTY_INVITES` 16, roles ⊆ members, leader ∈ members while live). `ContractRef(pub u64)`
(`ids.rs:49`) is **opaque** — the party never interprets it, and the contract never points back
(D-E6-5). The three seams the ruleset drives it through are authority-free by construction, because
the ruleset has already asked the authority question: `lock(ctx, id, by)` (`api.rs:64`),
`release(ctx, id, how)` (`api.rs:92`), `unlock(ctx, id)` (`api.rs:114`); the reads are `parties_of`
(`api.rs:134`) and `may_view` (`api.rs:150`), and every refusal renders `NO_SUCH_PARTY`
(`api.rs:49`). `Locked` refuses **every** membership change, so a substitution must amend the
contract and `unlock` first. One `+party` spec with nine switches, a 60 s expiry tick capped at 32
parties per fire (a `Locked` party never expires — its contract's deadline governs), and a boot that
drops a document disagreeing with its own key.

**`ruleset-shadowrun::economy`** (`src/economy/`, ~4 400 lines over nine modules).

- **`config.rs`** — the frozen `[shadowrun.economy]` block, `LifestyleTier` with the downgrade
  ladder that terminates at the free `street`, and §9's constants. Default `enabled = false`: the
  verbs register and answer "not enabled on this server" (E5c deviation 1).
- **`record.rs`** — `SrProfile` (`sr:profile:{obj}`) and `RunTemplate` (`sr:run:{id}`), both
  `#[serde(try_from = …)]` over a wire. The listing's wire refuses a `System` payer, an empty or
  over-long name, an empty outcome table, impossible slots, a negative amount, and a denormalized
  faction that disagrees with the board — so D-E6-4 cannot re-enter through a document.
- **`store.rs`** — the slot, the two document families, `put_profile`/`put_run` as the single write
  paths, a re-derived contract → listing index, and a `load` that is **total**: a corrupt profile or
  listing is skipped loudly and the world boots. The direction of each loss is deliberate and
  documented — a profile is re-derivable, and a listing is a *projection of a contract that still
  exists*, so losing one costs a board row and never an escrow.
- **`boot.rs`** — `run` (`:62`) registers `sr.nuyen` at scale 0 with `display = ¥:prefix` (`:88`),
  opens the two faucets and the sink, then **finds** the declared `sr.month` (`:188` — it never
  creates one) and reconciles: DL-07 adopts a self-identifying upkeep record and cancels one no
  profile claims (`:303`; with `lifestyle = false` it cancels nothing), and `reconcile_board`
  (`:418`) re-mirrors every listing onto its contract.
- **`lifestyle.rs`** — `set_tier` (`:162`), `open_upkeep` (`:215`), `pay_arrears` (`:290`),
  `on_obligation_changed` (`:383`), `miss` (`:450`), `apply_downgrade` (`:495`).
- **`board.rs`** — `RunSplit` (`:166`) + `table` (`:200`) + `even_split` (`:116`), `may_view`
  (`:273`), `post` (`:398`), `apply` (`:582`), `crew_accept` (`:641`), `crew_decline` (`:726`),
  `split` (`:760`), `withdraw` (`:820`).
- **`chargen.rs`** — `on_approved` (`:72`), one `mint` over a `MintSpec` (`:190`), `plan` (`:293`)
  and `commit_one` (`:354`).
- **`observer.rs`** — `SrBoardObserver`, exhaustive over all six `EconomicEvent` variants.
- **`commands.rs`** — `register` (`:51`), the board's own notice (`:277`), `run_post` (`:350`),
  `lifestyle_cmd` (`:753`), `nuyen_cmd` (`:830`), `sr_clock` (`:954`), `sr_migrate` (`:989`).

**The run, end to end, and which authority each step uses.**

| step | verb | authority |
|---|---|---|
| draft + offer | `+run/post` | the **payer is the issuer** (D-E6-19): personally, or through `scope=` an officer holding `contract.draft` on it (`resolve_issuer`) |
| escrow | (the same call) | the payer's own `accept_contract` — E4b2 places the consideration hold at *their* accept, so **no listing exists before its escrow** (answer 24). A refusal cancels the draft (M17) |
| apply | `+run/apply <id>=<party>` | the party's **leader**, gated first by `mush_party::may_view` |
| crew | `+run/crew <id>=accept` | the payer again, as the **author** of an A′ amendment: the crew join as parties, every row is rewritten to their own `SrProfile.account`s, and the payer's acceptance and escrow stand |
| bind | `@contract/accept` | each runner's own; the first locks the roster, the last activates |
| attest | `@contract/attest <id>=<row>` | the **named adjudicator**, who replaces every condition's own attestor set |
| settle | the economy sweep | `ContractAuth`; the board only mirrors what it hears |

**The outcome table as built** (D-E6-6, answer 7). Every row's payouts are `Share::Exact` and sum to
the escrowed total exactly, so `remainder_to` is inert and resolves to the payer's own account
(M18). Three row shapes: the first row pays `pay − fee` to the crew; a middle row pays half of that
(floored) to the crew and the rest back to the payer; the last row of a multi-row table pays the
`advance` to the crew and the **reserve** back to the payer under `ExpenseTreatment::Returned`. The
fixer's `fee` is a named share in **every** row. Before crewing the crew's share sits on the payer's
own account (M6); at `+run/crew accept` it is split across the roster by `even_split` — the floor
each, the remainder one minor unit at a time in roster order — so the sum is exact and **no
remainder reaches the fixer**. Shares naming one account are merged and zero shares dropped.

**The lifestyle downgrade rule as built** (D-E6-3, D-E6-23, M11). One fixed-amount obligation per
profiled character on `sr.month`, created through `create_obligation` under the actor's own debit
authority, `grace = 1`, `maximum_catch_up = 1`. The driver is `ObligationChanged`, and what acts is
the **delta**: a miss counts only when the record's `missed_total` is above the high-water mark the
profile already accounts for. `Grace` (the first miss) adds the period to `arrears`, bumps
`unpaid_periods`, sends one private notice (capped at three, then silent) and **downgrades
nothing**. `Delinquent` (the second) does the same and then replaces the record: cancel, drop one
tier, `create_obligation` at the new tier with `unpaid_periods` and `arrears` **carried**. A
forgiven period increments `forgiven_total` and never `missed_total`, so a downtime or clock-jump
backlog moves nothing; catch-up 1 makes one fire one attempt, so a ten-period backlog can never be
the second miss. `street` is free, so the ladder terminates. `+lifestyle/set` is refused while the
profile or the live record is in arrears, so a tier change can never wipe a miss; `+lifestyle/pay`
posts the arrears to `system:sr.lifestyle` under the player's **own** authority (keyed on the
`sr.month` ordinal) and only then reinstates.

**The one `mush-economy` addition: `cancel_contract`** (`contract_live.rs:4885`) — the fourth
programmatic seam, game-neutral, and the reason it exists is E6a's own: `+run/post` must cancel a
draft whose accept was refused (M17), `+run/withdraw` is the payer taking a listing down, and
neither may dispatch `@contract/cancel` from inside a handler. It admits a `Draft`, an `Offered`
contract, **and A′'s other face** — a live contract with no deliverables that nobody but its own
author has accepted, which is exactly a `Listed` run (S1 deviation 2). It returns whether the
contract actually reached a terminal state, so a refused refund is never reported as done. Four
tests in `e4b2.rs` (`e6a_cancel_contract_…`).

**Wiring.** `mush-party` in `server_modules_with` before the rulesets (`modules.rs:242`); the SR
board observer appended by `econ_observers` (`modules.rs:253`) **only when Shadowrun is enabled**;
`ruleset_modules(.., sr)` freezes `[shadowrun.economy]` into the ruleset; `ShadowrunCfg` /
`ShadowrunEconomyCfg` in `mush-config` (`config.rs:512,525`) with
`config_ext::shadowrun_economy_config` (`:250`) validating every word; and
`config_ext::shadowrun_cadences` (`:223`) + `economy_config_with` (`:78`) contribute the `sr.month`
stanza to `[economy.cadences]` **unless the operator declared one of that name** — so the economy
registers its `ClockProvider` at composition and creates or adopts the clock at boot, and
`@sr/clock` is show-only.

**Deviations from §3, with reasons.**

1. **`cancel_contract` is a fifth economy seam** (§2 listed four). Without it the ruleset had no way
   to honour M17 or `+run/withdraw` except by dispatching a command from inside a handler.
2. **`cancel_contract` admits an author-only live contract**, and `+run/withdraw` needs it: a
   one-party contract activates at its author's own accept (S1 deviation 2), so a `Listed` run's
   contract is `Active`, not `Offered`. The board's mirror therefore reads `Active` as `Crewed`
   **only when a crew is on the listing** (`boot.rs:473`, `observer.rs`). "A live agreement is not
   one party's to tear up" is a rule about other people's consent; here there is none to tear up.
3. **`+run/post` takes `fixer=` / `fee=` / `advance=` / `account=` / `days=` beyond §3.4's
   grammar**, and `+run/split` edits the `fee` and `advance` slots rather than a `<slot|who>`. While
   a run is `Listed` there is no roster to re-split, so those two are the only editable slots; a
   fixer named without a `fee` row is refused outright, because an unnamed remainder must never
   reach them.
4. **`RunTemplate` gains `applied: Vec<u64>`** (≤ 8). §3.1's shape had nowhere to hold a pending
   application between `+run/apply` and `+run/crew`; `accept` answers the first applicant unless an
   explicit party id follows.
5. **`@sr/migrate commit` adopts an approved character with nothing to mint**, opening the account
   and writing the profile. M13 asks for the adoption of a character whose profile records no mint;
   the zero-allotment case is the same shape, and refusing it would leave them outside the layer.
6. **`SrProfile` gains `unpaid_since`** — the `sr.month` ordinal the counters and the
   `+lifestyle/pay` idempotency key are keyed on (E5c T3: never wall time).
7. **A re-entered `on_world_loaded` tolerates a duplicate payload-handler registration**
   (`lib.rs:88`). A restart composes a fresh engine, so this is not a production path; it is what
   makes the boot reconciliation re-runnable, and therefore testable.
8. **`ShadowrunRuleset` registers no tick.** The board needs none: a listing's expiry *is* its
   contract's deadline (M8), swept by the economy, and the board hears the result through
   `ContractChanged`. `store.rs` keeps the tick constants for E6b.

**Handoffs.**

- **E6b** registers `with_canon_outcome_kinds(vec!["scene.outcome"])`; `+run/post` then admits
  `before`/`after`/`fact` rows (`ContractSpec::deterministic` is already the switch) and the board's
  `notice.kinds` stops being the constant `["attest"]`. `fence_rate_immediate` /
  `fence_rate_consign` / `fence_delay_secs` are parsed and inert, waiting for it.
- **E6c** gets `catalog_paths`, `chargen_shopping`, `quote_ttl_*` and `chargen_carryover_cap` — all
  parsed, all inert. Answer 17's carry-over cap is deliberately **not applied** and is read in
  exactly one place (`chargen.rs`'s `commit_one`, as `let _ =`), so turning it on is a one-line
  change beside a comment saying why it is off.
- **E7** gets `sr.run.participated` answered from `Settled { transaction }`, never from board state,
  and the account-distinctness verdict at the roster lock (doc 10 §15 R3, G10) — `+run/crew accept`
  already derives every payout account from the roster and refuses a crew containing the
  adjudicator.

**Verify fold (2026-09-05).** Twenty-seven findings from three adversarial lenses — disclosure and
bounds (D), money and consent (M), durability and replay (R). Twenty-five carry a
`vf_<lens><n>_…` regression test, **32 in all** (27 in `ruleset-shadowrun/tests/e6a.rs`, 2 in
`mush-party/tests/e6a_party.rs`, 2 in-crate in `board.rs`, 1 in `mush-economy/tests/e4b2.rs`); D9's
dead clause is covered by the existing membership tests and D11's probe cannot be reproduced from
any shipped verb — the table says why for each. The economy and the party stayed game-neutral; the two `mush-economy` additions
below are named and are about contracts and accounts, not about runs.

| item | what changed | test |
|---|---|---|
| **M1** | `RunSplit::table` with an empty roster emits one row per outcome paying the **whole escrow back to the payer** — no fee row and no advance row exist until a roster does, so the named adjudicator attesting a `Listed` run (which is `Active` — S1 deviation 2) settles a no-op instead of paying the fixer out of an uncrewed run (`board.rs::pre_crew_table`) | `vf_m1_an_adjudicator_settling_a_crewless_run_moves_nothing` |
| **M2** | `board::crew_accept` re-checks `party_has_live_run` **and takes the party lock before `amend_contract`**; `mush_party::lock` (which refuses anything not `Forming`) is the gate, and a refused amendment `unlock`s the party it had just locked | `vf_m2_one_party_cannot_be_crewed_onto_two_runs`, `vf_m2_a_disbanded_party_cannot_be_crewed` |
| **M3** | `SrProfile.missed_seen: u32` (`#[serde(default)]`) is the miss high-water mark **per record**, reset in `open_upkeep`; `unpaid_periods` widens to `u32` and only accumulates. The ladder no longer stops one tier below where it started | `vf_m3_the_ladder_keeps_walking_after_the_first_downgrade` |
| **M4** | `run_post` refuses `adjudicator=` naming the **fixer** or the **Johnson** (answer 22 as amended: distinct wherever a conflict of interest exists) | `vf_m4_the_adjudicator_is_never_the_fixer_or_the_johnson` |
| **M5** | `board::withdraw` refuses once a runner has accepted — `withdrawable` asks the *contract's* acceptances, not the board's mirror — and points at `+run/crew decline` or the deadline | `vf_m5_withdraw_is_refused_once_a_runner_has_accepted`, `vf_m5_a_listed_run_is_still_the_payers_to_withdraw` |
| **M6** | `lifestyle::replace_upkeep` opens the replacement **first** and cancels the old record second; a refused cancel pages (`EscalationSubject::Obligation`) and `reconcile_lifestyle` cancels any `sr.lifestyle` record the profile does not point at | `vf_m6_a_refused_replacement_leaves_the_profile_untouched`, `vf_m6_boot_cancels_a_lifestyle_record_the_profile_does_not_point_at` |
| **M7** | `chargen::plan` reads a character **with** an `SrProfile` and no mint as `Adopt(allotment)`; only a character with no profile at all is answer 18's `WouldMint(sheet value)` | `vf_m7_a_profiled_character_is_adopted_at_the_allotment_not_the_sheet` |
| **M8** | the crew check fails closed: `adjudicator_shape` reduces the consent surface to `Party(p)` or `Other`, and anything but one named party refuses the crewing | `vf_m8_only_a_single_named_party_is_a_usable_adjudicator`, `vf_m8_conditions_must_agree_on_one_party_when_no_adjudicator_is_named` |
| **R1** | `reconcile_board` releases the listing's party whenever the **mirrored** state is `Closed`, including `now == was` — the shape a dropped board/party write leaves | `vf_r1_boot_releases_the_party_of_a_listing_that_is_already_closed` |
| **R2** | `reconcile_parties` — a boot pass over live `Locked` parties: the ruleset resolves the opaque `ContractRef` it handed over and releases every one whose contract is gone or terminal (log + one page per orphan, capped at 8). `mush-party` stays money-blind | `vf_r2_boot_releases_a_party_orphaned_by_a_skipped_listing` |
| **R3** | `chargen::already_minted` asks the **idempotency index** (`mush_economy::idempotency_resolved`) for `sr.chargen/{obj}` and `sr.migration/{obj}`, so a corrupt profile cannot buy a second allotment | `vf_r3_a_corrupt_profile_never_mints_a_second_allotment` |
| **R4** | `personal_account` (both copies) filters `AccountStatus::Active`, and `boot::reconcile_accounts` re-derives `SrProfile.account` when the account it names is not | `vf_r4_a_closed_account_is_re_resolved_at_boot` |
| **R5** | `miss` and `pay_arrears` refuse to act with no `sr.month` ordinal (the counters and the key are keyed on it and nothing else), and the pay key carries the arrears amount as a second discriminator | `vf_r5_paying_arrears_with_no_cadence_is_refused`, `vf_r5_the_pay_key_discriminates_on_the_amount` |
| **D1** | `SrEconomyStore.live: BTreeSet<RunId>` drives the caps, the per-party rule and the board page; a closed listing is `Del`'d (`store::del_run`) with a 32-id `recent_closed` ring for `+run/show`; `visible()` walks at most `BOARD_PAGE_VISIT` (256) live records from an `after=` cursor and returns a `BoardPage` with its own continuation (D-LBK-5: bounded on VISITED) | `vf_d1_a_post_withdraw_loop_leaves_no_stored_listings`, `vf_d1_the_board_page_is_bounded_on_records_visited` |
| **D2** | `MAX_OUTCOME_NAME_BYTES` (64) enforced in `mush_economy::check_structure` — so no producer, command or seam or document, can freeze an oversized row — plus `commands::outcome_names` and `RunTemplateWire::try_from` | `vf_d2_an_oversized_outcome_row_name_is_refused_at_post_and_on_load`, `vf_d2_the_economy_refuses_an_oversized_outcome_row_from_any_producer` (e4b2) |
| **D3** | `resolve_account` filters the compartment's candidates through `may_view_account` and refuses with the single `"No such account."` unless exactly one viewable account remains | `vf_d3_a_multi_account_compartment_hides_like_an_unknown_scope` |
| **D4** | `resolve_faction`'s numeric branch runs `faction_listed` too | `vf_d4_a_hidden_faction_hides_by_id_as_well_as_by_name` |
| **D5** | the fee-row rule is asked **before** the fixer's account (it is a fact about the command, not the person), and a fixer with no wallet renders as `"No such fixer."` — the unresolvable-name text — so `fixer=` is no longer a free banked/unbanked probe | `vf_d5_a_banked_and_an_unbanked_fixer_answer_alike` |
| **D6** | `mush_party::lock` prunes the outstanding invitations (a locked party can neither accept one nor withdraw one, and `may_view` admits an invitee), and `show_lines` never renders the `ContractRef` to a non-staff viewer | `vf_d6_the_lock_prunes_outstanding_invitations_and_hides_the_party`, `vf_d6_a_members_view_names_no_contract_id` |
| **D7** | `by_obligation` / `by_account` indexes on `SrEconomyStore`, maintained in `put_profile` and re-derived at boot; `on_obligation_changed` resolves its subject in O(log n) | `vf_d7_the_indexes_agree_after_every_profile_mutation`, `vf_d7_a_population_wide_period_boundary_bills_every_profile` |
| **D8 / R6** | `RunNotice.kinds` (≤ 8 × 32 B), `RunNotice.window` (≤ one year) and `Mint.key` (≤ 128 B) capped in their wires (`record::check_notice`) | `vf_d8_oversized_notice_fields_and_mint_keys_refuse_to_load` |
| **D9** | `board::may_view`'s `Faction` arm drops the dead `membership_visible(viewer, false, m)` clause (always true for `viewer == m.character`), with a comment saying why | covered by the existing membership tests |
| **D10** | `@sr/profile` renders arrears through `money_on` | `vf_d10_the_staff_profile_renders_arrears_as_money` |
| **D11** | `mush_economy::resolve_org_visible` — hidden ⇒ unresolvable unless staff/member/grantee — behind `parse_principal`, so **every** verb that takes an org name answers a hidden org exactly as it answers an unknown one | no end-to-end test: nothing in the shipped surface writes `Organization.hidden` (only `render_principal` reads it), so the probe cannot be reproduced. Preventive |
| **D12** | `nuyen_lines` emits one `NO_NUYEN` line for an unviewable account *and* for a viewer who may see neither account nor sheet, so silence stops meaning "banked" | `vf_d12_an_unviewable_account_and_no_account_read_alike` |
| **D13** | `EconomySnapshot::accounts_of(owner)` (owner index, `put_account`-maintained), used by both `personal_account`s | `vf_r4_a_closed_account_is_re_resolved_at_boot` asserts the index agrees; `vf_d7_…` covers the rest |

**Two `mush-economy` additions, both game-neutral.** `idempotency_resolved(ctx, &key) -> bool`
(`api.rs`) — the index is the authority on "did this one-shot post land", and a caller's own record
of it is a cache. `EconomySnapshot::accounts_of(owner)` plus `resolve_org_visible` and
`MAX_OUTCOME_NAME_BYTES` (`commands.rs`, `snapshot.rs`, `contract.rs`). None of them says the word
"run".

**Deviations from the fold's own brief.**

1. **D5 keeps `fixer=` usable.** The brief's literal fix — resolve the fixer's account *through*
   `may_view_account` — would have made `fixer=` unusable by every player, because no player may
   view another player's personal account and there is no route that grants it. The oracle it names
   is closed instead at its root: the **free** probe was `fixer=<who>` with no `fee=`, which answered
   "no nuyen account" or "needs a fee row" depending on the target. The fee-row rule now runs first
   and is target-independent, and a missing wallet answers with the unresolvable-name text.
2. **M6 pages on `EscalationSubject::Obligation`**, not a new ruleset-shaped subject. The stuck
   record *is* the subject, the ladder's per-subject dedupe then means one page per stuck record,
   and the recommended command (`@obligation/cancel #n`) names it.
3. **D7's population test is 40 profiles, not 1,000, and asserts correctness rather than a visited
   counter.** A counter on the observer path is production code that exists only for a test; the
   index itself is the claim, and `SrEconomyStore::indexes_agree()` is what asserts it.
4. **A closed listing's document is deleted** (D1), so `run_state` in the acceptance suite reads
   "gone" as `Closed` and `+run/show` answers "closed" from a bounded ring of ids rather than from a
   record. Three shipped tests moved with it: the fixer's fee row is now asserted after crewing (M1),
   and two `mush-party` tests assert the pruned invitation (D6).

---

## 4. E6b — outcomes, fences, custody

> **`file:line` cites in this section are historical.** They were exact when 0.3 was written and
> predate the whole-tree reformat (`91208ab`) and E6a's own folds; the **symbol named beside each
> one** is the citation, and §4.6 carries the as-built anchors. They are deliberately not
> re-numbered — a line number that is re-derived every cut stops being evidence of anything.

### 4.1 `scene.outcome` — the producer E4c's fold owes the economy

`mush-scenes` today produces exactly one canon record: `scene.concluded`
(`crates/mush-scenes/src/lib.rs:301-330`), `Restricted`, `entities = participants`,
`source: scene:{uid}`. E4c's fold disqualified it as evidence and left `with_canon_outcome_kinds`
empty on purpose (`crates/mush-server/src/modules.rs:151-157`), so every `canon:` attestor set is
refused at `@contract/offer` today. The tree proves the fold right and sharpens it: **any connected
player may `+timestop` in an unscened room (`crates/mush-scenes/src/commands.rs:102-104`) and is made
`facilitator` by that act (`commands.rs:160`)**, and `+timestop/end` needs only `driver_scene` —
facilitator or staff (`commands.rs:216-236`, `:389`). A seal signed by "the facilitator" alone would
be exactly as forgeable as `scene.concluded`. So the seal is a **conferred** authority:

```rust
// added to SceneRecord (crates/mush-scenes/src/data.rs:111-154), both #[serde(default)]
pub sealer: Option<ObjId>,          // conferred by staff; None means this scene CANNOT be sealed
pub outcome: Option<SceneOutcome>,  // written once, terminal, never edited
pub struct SceneOutcome {
    pub name: SmolStr,              // matches an outcome row ("success"/"partial"/"failure")
    pub subjects: Vec<ObjId>,       // named AT SEAL TIME — not copied from `participants`
    pub note: String,               // ≤ MAX_OUTCOME_NOTE_BYTES; chronicle text, never authority
    pub sealed_by: ObjId,  pub sealed_at: i64,
}
```

- **`@scene/facilitate <scene>=<who>`** (staff, the `Role::Royalty` check already at
  `commands.rs:46-48`) confers `sealer`. **`+scene/outcome <name>=<subject…> run=<id> [note]`** seals, only
  from `sealer` or staff, only once, only on an `Active`/`Paused`/`Closed` scene. A scene with no
  conferred sealer produces no outcome at all — fail-closed exactly like the empty table it replaces.
  (`+scene` is `ArgShape::None` with no switches today, `commands.rs:672-682`; the switch set is new.)
- The commit is a second `CanonDraft` beside `emit_scene_concluded`: `kind: "scene.outcome"`,
  `source: run:{contract id}` named by the sealer — a `scene:{uid}` is unknowable when the run is
  offered, and the economy compares `source` as an opaque string (`canon_set_accepts`,
  `contract.rs:1746-1753`; the `scene:{uid}` at `attest.rs:32-34` is a doc comment — M14),
  `entities: subjects ∪ {sealed_by}`, `scope: Restricted`,
  `detail: { scene, outcome, sealed_by, round }`. **`ic_time` is `scene_ic_time()`, after the seal
  has raised `ic_floor` to `ctx.game_now_unix()`** (OPEN 3, as built): one rule instead of two, the
  same value either way, and the general floor doc 10 §15/6A asks for rather than a workaround
  beside it. A long timestop's scene clock advances only `round_seconds × (rounds − 1)`, so without
  the floor a stale stamp would chronicle a settlement before events it followed; E6b owns the
  floor, cut 6 owns the raise, and the monotonicity test is E6b's.
- **`entities` is named, not inherited.** `scene.concluded` copies `participants` (`lib.rs:319`),
  frozen at scene creation with no add path (`commands.rs:119-141`; a later arrival is only logged
  `Joined`, `lib.rs:552-561`). The economy requires the record's entities to name or act for a
  contract party (`attest.rs:217-226`, `names_a_party` at `attest.rs:249-255`), so a runner who joined
  mid-scene would silently fail to match. Naming subjects at seal time makes the match reliable *and*
  is what answer 6 asks for.
- `mush-server` then registers `with_canon_outcome_kinds(vec![SmolStr::new("scene.outcome")])`
  (`modules.rs:157`). `scene.concluded` is never registered.

**Review amendment (0.2, re-grounded at 0.3 — M14) — the seal is not a per-run staff gate.** A
staff-conferred `sealer` would put one staffer in front of every run (08a decision 25, the
staff-scarce rule), and the run's **named adjudicator** (consented at offer — answer 6 — and never a
party or the poster of that run: answer 22) already ends the run by `@contract/attest`, replacing
every condition's attestor set (D-E6-8; the E6a route, M2). The seal is the *canon* route beside it,
signed by a staff-conferred `sealer` or staff. The seal names its run: `source: run:{contract id}`
(a `scene:{uid}` is unknowable at offer), `entities = subjects ∪ {sealed_by}`. Who may seal is the
producer's gate (adjudicator identity is not visible to `mush-scenes`, which has no economy edge —
§2); the economy checks kind, source and party intersection as today. A `detail`-bearing
`CanonRecorded` (today it carries `{canon_id, kind, entities, source}` only,
`mush-core/src/module.rs:523-533`) or DL-09's read seam is what an economy-side `sealed_by` check
waits on. Staff conferral remains for scenes no contract names.

### 4.2 The four settlement routes, one alphabet (answers 6, 19)

| Route | The condition E6b writes | Supplied by |
|---|---|---|
| Sealed scene outcome | `Attestation { by: Canon { kind: "scene.outcome", source: "run:{contract id}" } }` — source **mandatory** (`contract.rs:1553-1557`) | a conferred sealer or staff (§4.1, M14); the adjudicator's own route is the next row |
| Authorized attestation | `Attestation { by: Party(p) \| Capability{scope,capability} \| Staff }` via `@contract/attest` | the adjudicator named at offer |
| Deterministic | `Fact` / `TitleHeld` / `DepositInto` / `Before` / `After` — E6b onward: E6a has no `sr.*` provider, and a bare `Before` row is `Ready` at the last acceptance (M2, D-E6-20) | the journal, or an `sr.*` `FactProvider` |
| Composite | several of the above; outcome rows index the ones deciding *which* result | — |

The **named adjudicator replaces** each condition's own attestor set, for canon and people alike
(08c §5.1 dev. 4, §6.1 dev. 3), so `+run/post adjudicator=` is what actually binds who may end the
run, and the runners consent to it at acceptance. **Prose settles nothing:** a canon record with no
matching typed condition attests nothing whatever it says, and an LLM ST's proposed outcome reaches
the world only as a `+scene/outcome` a conferred human seals (answer 23).

### 4.3 Fences, custody, and title (answers 13–15)

```text
+fence/quote <fence>=<obj> [immediate|consign]   a deterministic FenceQuote on a declared clock
+fence/accept <quote>                            drafts + offers; the FENCE accepts FIRST (escrow)
+fence/deliver <id>                              refused unless the contract is already Active (D-E6-9)
+fence/list   +fence/show <id>
@asset/stolen <obj>=<public|restricted|covert> [reason]        the typed report (economy; §2 item 2)
```

- **Escrow before custody (answer 15).** `+fence/deliver` refuses while the contract is `Offered`,
  because E4b2 custody-locks a `DepositInto` object only for an `Active`/`ReadyToSettle` contract
  (08c §5's boot re-derivation) — a seller delivering early would hand over an unlocked, unpaid
  object. The fence's `Consideration` is escrowed at their accept; only then does the runner deliver.
- **Immediate vs consignment.** The immediate quote settles on `DepositInto` alone for less; the
  consignment adds `After { condition: <deposit idx>, delay, clock: RealUtc }` for more — exactly the
  shape the shipped fence test drives (`crates/mush-economy/tests/e4b2.rs:778-800`, with
  `@contract/condition {id}=after 0 600`). `fence_delay_secs` bounds the delay to 24–72 h and the
  offer shows it. Notices fire only on custody, settlement, cancellation and dispute (answer 15).
- **Title ≠ possession (answer 13).** The deliverable is `DepositInto`, **never** `TitleTransfer`:
  the fence buys custody. `Encumbrance::Disputed` and the original title survive; the settlement's
  `AcquisitionRecord` is evidence, not guilt (answer 14).
- **Deterministic pricing, strict visibility (answer 13).** `FenceQuote { id, fence, seller, object,
  catalog, offer: Amount, basis: Vec<PriceFactor>, settlement: Immediate | Consign { delay },
  expires: EconomicDeadline, observed: QuoteObserved }` — `RevisionSet` is a `mush-core` type and
  not an economy one, so `observed` is the pair of per-record `revision: u64` stamps the economy
  actually has: the asset's own and the fence policy's own (as built, §4.6). `PriceFactor` is computed **only** from what
  the fence may observe (the asset record under `may_view_*` rules, the seller's presentation, the
  catalog baseline) plus a declared legality/demand table. A provenance fact the fence may not see
  contributes **nothing — not a discount** — so a low quote is never an oracle for hidden state.

**Re-grounded at 0.3 (M15, M19, G3, G4).** A staff-built NPC fence has no officer, and an org party
accepts only through `acts_for` holders or staff (`contract_live.rs:337-345`), which would put staff
in every deal: the fence accepts **programmatically** through DL-11's vendor accept
(`with_party_caps`), an E6b item (§2 item 7). A vendor org's consignment deposit is stranded at
unwind today — `refund_plan` returns deposits to `Object` depositors only and `authorize_vault`
withdraws only to `Object` (E4b2-D28) — so the DL-03 seam (contract-scoped `Deposit` authority for
the counterparty vault plus a `Withdraw` destination of an org-owned vault's backing;
`destination_ok` admits it) lands before consignment ships. `Organization.hidden` **is** honoured where the economy
renders since E6a step 1's fold D7 (`render_principal` over `may_see_org`); what was missing was the
*listing-absence* rule and the scenario-12 test, and both land here. An org party, vendor or fence
renders through one `render_org_listed(ctx, org)` — a thin named wrapper over that pair, not new
logic — and a hidden fence's **listings** are absent for non-members through `may_list_org`, never
"No such". The fence's money is sound as
written: its escrow refuses on `InsufficientAvailable`, so "stops quoting" is automatic (M19).

### 4.4 Augmentations (answer 12, minimal)

`schema.rs:275-279` stores `augmentations` as a free-text `Map<Number 0..600>` valued in Essence ×100,
and `essence_from_augmentations` (`derive.rs:211-217`) runs **only** from chargen finalize
(`chargen.rs:645-646`) — `apply_derived` deliberately does not touch Essence (`derive.rs:184-187`),
so a post-approval `@sheet/set augmentations:` already desyncs Essence today. E6b therefore ships
only: an `InstalledAugmentation { object, catalog, grade, essence_cost, clinic, acquisition:
AcquisitionRecord, installed_at }` on `SrProfile`; the rule that **a service contract's settlement
never writes the sheet**; and the refusal that keeps Karma costs on `+karma/spend`'s job flow
(`crates/ruleset-shadowrun/src/karma.rs:146-218`, queue `shadowrun-karma`, handler registered
`lib.rs:272-281`; no karma↔nuyen path exists, `karma.rs:360-442`). The atomic installation action —
and moving the Essence recomputation onto every write — is the later clinics slice.

### 4.5 E6b tests (~27 planned; **67 built**, §4.6)

**`scene.outcome`:** a scene with no conferred sealer cannot be sealed and its contract never settles; a non-sealer non-staff player is refused; a second seal is refused; `kind`/`source`/`entities` are exactly what the observer matches and a seal naming another run attests nothing (M14); a runner who joined mid-scene is named in `subjects` and matches, one omitted does not; `scene.concluded` from the same scene attests nothing; a `canon:scene.outcome` set with no source is refused at offer (`contract.rs:1553-1557`); the seal's `ic_time` is the world IC clock, not the frozen scene clock; a sealed outcome selects the matching row and pays it; two rows satisfied at once reads `Undecidable` and disputes rather than freezing.
**Routes:** each of the four settles; a composite needs every gating condition plus one row; the adjudicator replaces a runner's own attestor set; an unregistered canon kind is refused at offer.
**Fence:** `+fence/deliver` refused while `Offered`; the consignment matures only after the delay; the immediate sale settles at once; the seller's title and `Disputed` encumbrance survive settlement; two worlds differing only in hidden state produce identical quotes; an expired quote is refused (scenario 29's shape); a hidden fence org renders as "an organization" to a non-member and its quotes are absent (G3, scenario 12); the fence accepts with no officer connected (M15); an org fence's consignment deposit comes back at unwind (DL-03).
**`@asset/stolen`:** the owner, an insurer and an authority may file, a bystander may not; covert visibility is invisible to the thief; a later purchase generates evidence and does not clear the claim.
**Augmentation:** settlement writes the record and not the sheet (assert `sheet:shadowrun` unchanged across every test in the file).
**Scene clock (08i §7 Q9, frozen with K):** `SceneClockProvider` (`crates/mush-core/src/clock_provider.rs`) is read-only and exposes no generation bump, so one explicit monotonicity test drives a `TurnBased` scene through `+timestop` start → `/next` advance → `/pause` (and the boot-time `PauseReason::ServerRestart`) → `/resume` → `/next` advance, and asserts `DurableSceneClock::now` (`crates/mush-scenes/src/lib.rs`) is non-decreasing across the whole sequence within `SceneRecord.clock_generation` 1 (`crates/mush-scenes/src/data.rs`); an `ic_floor` raise (E6b's floor over `SceneRecord.ic_started_at`) never changes `clock_generation`, which lasts the record's lifetime; and any implicit generation mutation is rejected — a read at generation 2 is `ClockProviderFailure::StaleGeneration`, and no E6b verb or boot path writes `clock_generation`.

**Hazards on this axis.** *Prose settlement:* a canon record with no matching typed condition is inert, and the seal is conferred. *Fence laundering:* the deliverable is custody and `TitleTransfer` is never in a fence template. *Consignment custody races:* the deliver-refusal before `Active`, the `DepositInto` lock while live, and E4b2's unwind returning each recorded deposit from the journal's `last_custody` (08c §5.1).

### 4.6 As built (2026-09-05)

E6b on top of E6a and its verify fold (`ff3f581`). Nothing Shadowrun-shaped entered `mush-economy`,
`mush-scenes`, `mush-canon` or `mush-core`; `mush-scenes → {core, canon, proto}` is unchanged and has
**no** economy edge. All money still moves through E4 under an authority; every persisted addition is
a `TryFrom` wire with documented caps; every tick, observer, boot and sweep path is an exhaustive
match with no `?`, `unwrap`, `expect` or panicking index. Both digest pins (E2 `9c917eee…`, E3-shaped
`b780e3a0…`) are green.

**B1 — one masking renderer, and hidden organizations render.** `pub fn render_account(ctx,
AccountId, noun) -> String` (`mush-economy/src/commands.rs:591`) is the one renderer over
`may_view_account`; `mask_account` is a one-line alias so no caller outside the crate had to move.
The history collapse is a second helper on the **same** predicate — `render_account_entry(ctx, id,
instrument, amount) -> Option<String>` (`:626`) plus the `MASKED_ACCOUNTS` token (`:617`) — and
`@econ/history`'s inline copy is gone. `render_org_listed(ctx, org)` (`:665`) is a thin named wrapper
over E6a's `render_principal`/`may_see_org` pair, and `may_list_org(ctx, org)` (`:678`) is that pair's
predicate, made public so a ruleset can **omit** a hidden vendor's listings rather than refuse them.
`may_view_asset` is `pub` (`:521`). The economy owns the render; the ruleset owns the absence.

**B2 — `AssetOp::ReportStolen` + `@asset/stolen`.** The variant (`plan.rs:147`) takes **ordinal 8**
(`plan.rs:195`); no existing ordinal moved, which is what keeps every stored idempotency record for a
title or custody post resolving. The effect is a plain `Encumbrance::Disputed` **plus** a bounded
`StolenReport { by, visibility, reason, at }` on `EconomicAsset.stolen` (`record.rs:568`,
`#[serde(default)]`, journal authority inside `AssetEffect.post`) — so insurance and salvage keep
consuming exactly the claim they consume today (08f §7 item 5). `StolenVisibility::{Public,
Restricted, Covert}` (`record.rs:583`) is read by `may_view_stolen_report`
(`commands.rs:3128`): `Restricted` needs `may_view_asset`, `Covert` is the reporter (through
`acts_for`) or staff. Authority relaxes E3 invariant 19 for exactly three (`api.rs:430`): the title
owner, a holder of the new **`asset.insure`** capability on the asset's compartment (added to
`CAPABILITIES`, `engine.rs:127`), and staff. The **claimant is derived**, never input — the asset's
own title owner (`commands.rs:3072`) — and the report is written **once**
(`EconomyError::AlreadyClaimed`, `engine.rs:1123`). The verb is a switch on the existing `@asset`
spec, so `CommandSpec::name` is unchanged and `legacy-handlers.toml` needs no stanza for it.

**B3 — DL-03: an org or system depositor's deposit comes home.** One router,
`deposit_returns(c, snapshot) -> Vec<DepositReturn { deposit, to, into_vault }>`
(`contract.rs:2515`) over `owner_home(snapshot, owner)` (`contract.rs:2568`, the lowest-id `Active`
vault that principal owns whose backing is a single world object). `refund_plan` builds its
`CustodyOp::Withdraw`s from it (`contract.rs:2600`) and `ContractAuth::unwinding` resolves the same
routes into `ContractAuth.homes` (`contract.rs:2790`), so the plan and the authority cannot disagree
about where somebody's property may go. Three gates were opened, and every one of them is
**authority-shaped, never cause- or key-shaped**:

- a new `AuthEval::authorize_vault_home(object, to)` (`engine.rs:250`), `false` by default and for
  `WorldAuth`, `true` only from `ContractAuth::unwinding` for a deposit that contract recorded
  (`contract.rs:2996`);
- the engine's withdrawal branch consults it before refusing a vault backing as a destination
  (`engine.rs:1500`), and when it answers `true` it still proves the destination vault is `Active`
  and its backing usable;
- `ContractAuth::authorize_vault` admits that same `(object, to)` pair (`contract.rs:2965`).

`returnable` (`contract_live.rs:3560`) asks the same router, and a deposit with **nowhere to go** is
**paged** — `page_stranded_deposit` (`contract_live.rs:3524`) names the principal and the
`@vault/create` that fixes it, on the contract's own escalation subject so the ladder dedupes to one
page per stuck unwind. `MAX_DEPOSITS` stays **1** (DL-15). Two consequences worth naming: the plan's
actor is then a non-object principal, so `run_custody_effects` (`api.rs:1336`) hands the provider the
destination backing as the `CarryAttempt` mover — the org's own vault takes its property back — and
it still refuses every other non-object actor outright.

**B4 — DL-11's vendor accept.** `EconomyModule::with_party_caps(Vec<(PrincipalId, PartyCap)>)`
(`module.rs:995`) is the **seventh** wiring-frozen table, bounded at `MAX_PARTY_CAPS` 64, keyed by
`principal_token` (`contract.rs:1030`) because `PrincipalId` is `Hash`-but-not-`Ord`. `PartyCap` has
exactly one variant, `Accept` (`contract.rs:1013`). It is reachable **only** from
`accept_contract_as(ctx, id, party)` (`contract_live.rs:4849`) — never from `@contract/accept` — and
it grants no money authority: `accept_core_as` (`contract_live.rs:1799`) runs the command's own core
and places the escrow with **the accepting principal as the plan's actor**, so the hold posts under
that account's own debit authority and refuses on `InsufficientAvailable`. That is the whole of
"a fence that runs out stops quoting" (M19, D-E6-15). The composition root builds the table from
`[shadowrun.economy].vendor_orgs` (`modules.rs:266`), ids only.

**B5 — canon evidence survives the tail and the reboot; refusal telemetry.** DL-09 ships as a
**bounded read seam**: `mush_canon::records_for_source(state, source, limit)`
(`mush-canon/src/lib.rs:200`, capped at `MAX_RECORDS_PER_SOURCE` 8) answers from the ledger's open
tail — which is exactly what a restart rehydrates — and `mush-server`'s `CanonEvidenceReader`
(`econ_canon.rs:246`) bridges it into the economy's own `CanonEvidence` trait (`attest.rs:196`), so
`mush-economy` still has no `mush-canon` edge. `rescan_canon_sources` (`attest.rs:230`) asks it for
every `source` a live contract's own `Canon` condition or adjudicator names, bounded twice
(`MAX_CANON_SOURCE_LOOKUPS` 32 × `MAX_CANON_RECORDS_PER_SOURCE` 8); it runs from `@econ/unfreeze`, a
contract's activation, and **once per boot** from the first sweep fire
(`sweep.rs:117`). The canon tail now carries each record's **`entities`**
(`module.rs:236`, capped at `MAX_CANON_ENTITIES` 64), so `rescan_canon_tail` passes `Some(entities)`
and the recovery path runs the same party-intersection gate the live path runs — it is no longer
"source exactness alone carries". C16 ships as `EconomyStore::note_refusal`/`refusal_tallies`
(`module.rs:478`), a RAM-only per-actor `{hidden, other, first_at, last_at}` counted at the two
refusal renderers (`commands.rs:799`, `:852`) and at `refuse_contract_offer`, bounded at
`MAX_REFUSAL_ACTORS` 256 by evicting the least recently refused, and surfaced by `@econ/alerts`
(`commands.rs:5054`). It is telemetry: nothing reads it to decide anything, and it is deliberately
not persisted.

**B6 — the conferred sealer, the outcome, and the IC floor.** `SceneRecord` gains three
`#[serde(default)]` fields (`mush-scenes/src/data.rs:283`): `sealer: Option<ObjId>` (**`None` means
this scene cannot be sealed** — fail-closed for staff too), `outcome: Option<SceneOutcome>` (written
once, terminal, never edited) and `ic_floor: i64`. `SceneOutcome { name, subjects, note, sealed_by,
sealed_at }` has its own `TryFrom` wire (`data.rs:61`) with `MAX_OUTCOME_NAME_BYTES` 64,
`MAX_OUTCOME_NOTE_BYTES` 240 and `MAX_OUTCOME_SUBJECTS` 32; `SceneRecord` itself stays raw
`Deserialize` (OPEN 5 — a v3 migration for a cosmetic win), and its new invariants live in
`validate_clock` (`data.rs:325`): a sealed outcome needs a conferred sealer, the floor is
non-negative, and the outcome's own caps are re-checked. `scene_ic_time()` and
`checked_scene_ic_time()` clamp to `ic_floor` (`data.rs:290`, `:305`); `raise_ic_floor`
(`data.rs:352`) is **monotonic** and never touches `clock_generation`. Because all three fields are
`#[serde(default)]`, an old document
loads with `sealer: None`, `seal_run: None`, `outcome: None`, `ic_floor: 0`, which is exactly the
pre-E6b behaviour — so **forward** no migration is needed, which is the half OPEN 5 asked about. The
fold added a **v3 identity** migration anyway, for the other direction (R3): the version stamp is
what makes the runner refuse a rollback that would erase a terminal seal.
`SceneRecord` gains a fourth `#[serde(default)]` field in the fold, `seal_run: Option<u64>` — the
run staff bound this scene to, opaque here because `mush-scenes` has no economy edge. New verbs:
`@scene/facilitate <scene>=<who|none> [run=<id>]` and `@scene/icfloor <scene>=<unix|now>` (staff,
`Role::Royalty`), and `+scene` — `ArgShape::None` with no switches before — becomes `ArgShape::EqPair`
with one switch, `+scene/outcome <row>=<subject>[,<subject>] run=<id> [note=<text>]`. Rule 2
(08i §4) is enforced at **both** ends by `is_sealable_principal` (`commands.rs:648`): only an
`ObjKind::Player` may be conferred the seal, and the same question is asked again at seal time, of the
sealer *and* the actor — nothing lets an NPC's completion seal a settlement.

**B7 — the seal is canon, and only the producer gates it.** `emit_scene_outcome`
(`mush-scenes/src/lib.rs:461`) is a **second** `CanonDraft` beside `emit_scene_concluded`, never a
change to it: `kind: "scene.outcome"` (`KIND_SCENE_OUTCOME`), `source: run:{contract}#{row}` — the contract **staff bound to this scene** at conferral, and the
row the seal answers (both the fold's, M2/M5) — `entities: subjects ∪ {sealed_by}` **named at seal
time**, `scope: Restricted`, `detail: { scene,
outcome, sealed_by, round }`, and `ic_time: rec.scene_ic_time()` after the floor was raised (OPEN 3).
`mush_canon::commit` is terminal and pins the deferred prefix, so it is the **last** mutating act on
the path. `mush-server` registers exactly `vec![SmolStr::new_static(mush_scenes::KIND_SCENE_OUTCOME)]`
(`modules.rs:210`); `scene.concluded` is **never** registered. What the economy checks is unchanged
and is stated out loud: **kind + source + party intersection, and nothing else** — there is no
`sealed_by` check, because `WorldEvent::CanonRecorded` carries no `detail`, so who may seal is the
**producer's** gate. Three stale doc sites now say so (`attest.rs:31-39`, `contract.rs:330`,
`module.rs:1042`).

**B8 — `+run/post` admits the other three routes.** `RunRoute::{Attest, Canon, Before, After, Fact}`
(`board.rs:349`) with `condition(adjudicator)`, `label()` and `is_deterministic()`; `conditions()`
(`board.rs:452`) lays **one condition per outcome row, then every gate**, so row `i` is decided by
condition `i` plus `RunSplit::gates` — which is exactly "a composite needs every gating condition plus
one row". `evidence_of` (`board.rs:487`) makes the permitted-evidence list the kinds the surface
actually uses instead of E6a's forced `[Attestation]` (there is **no** `EvidenceKind::Canon`: a canon
row's evidence is `Attestation`, filled by `TypedCondition::evidence_kind`), and `deterministic`
follows the routes. `notice.kinds` (`board.rs:470`) is derived from them and is no longer the constant
`["attest"]`. Grammar: `route=<row>:<spec>|<row>:<spec>` and `gate=<spec>|<spec>`, with
`<spec>` one of `attest`, `canon:<kind>:<source>` (the source is the **remainder**, because a seal's
source carries a colon of its own), `before:<unix>`, `after:<row>:<secs>`, `fact:<provider>:<kind>:<id>`
(`commands.rs:394-513`). `run:self` resolves to the id the offer is about to mint
(`board.rs:590` over `EconomyStore::peek_next_contract`) — a canon-routed run has to be able to name
itself, because the seal's source is `run:{contract}` and the condition freezes at offer.
`AdjudicatorShape` gains **`Impersonal`** (`board.rs:1101`): a canon set and every deterministic
condition name no *person*, so there is no conflict of interest for the crew check to find and a
routed run is crewable; a `Capability` or `Staff` set is still the fail-closed `Other`. The named
adjudicator still replaces every condition's attestor set, and `+run/post` therefore does **not**
default one onto a routed run (`board.rs:519`).

**B9 — the fence.** `crates/ruleset-shadowrun/src/economy/fence.rs` plus a third and fourth document
family on `store.rs`'s exact pattern: `sr:fence:{id}` and `sr:quote:{id}`
(`fence_record.rs`), the `put_fence`/`put_quote`/`del_quote` single write paths
(`store.rs:415-480`), a **total** `load` that skips a corrupt document loudly and boots anyway
(`store.rs:640-700`), the allocator floors in `sr:meta`, and DL-07's identity rule. Commands:
`+fence/list  +fence/show <quote>  +fence/quote <fence>=<obj> [immediate|consign]
+fence/accept <quote>  +fence/deliver <quote>`, plus the staff `@sr/fence <org>=<vault> [name=]
[demand=]` — a switch on the existing `@sr` spec.

*The price basis, as decided (OPEN 1, D-E6-11).* Three declared inputs and nothing else: the
source-controlled `[shadowrun.economy.fence_baseline]` reference price keyed on the asset's own
`CatalogRef.code`, the settlement rate in permille (`fence_rate_immediate` 300 /
`fence_rate_consign` 600), and the fence's own declared `demand` adjustment. An object whose asset
record carries **no `catalog`** cannot be quoted — refused, never guessed — and the price is never
derived from the `AcquisitionRecord`, which is provenance the fence may not see. `price`
(`fence.rs:121`) reads the catalogue code and nothing else about the asset: not the encumbrances, not
the `StolenReport`, not the acquisition, not the title owner. **A hidden fact contributes no factor
at all**, so two worlds differing only in hidden state produce byte-identical quotes and the basis a
seller reads is the whole of the arithmetic.

*The flow.* `+fence/quote` stores a `FenceQuote` with an expiry on `RealUtc` and `observed { asset,
fence }`; a lapsed window or a moved revision refuses **deterministically** rather than re-pricing
(`is_current`, `fence.rs:398`). `+fence/accept` offers with the seller as issuer and lets the
**fence accept first** through B4 — that accept is what places the escrow — leaving the contract
`Offered`. The seller binds with `@contract/accept`, and only then may `+fence/deliver` hand custody
over: it refuses anything that is not `Active`/`ReadyToSettle` (D-E6-9), which covers `Disputed` too,
since the `DepositInto` lock does not cover it either. The deliverable is the deposit;
**`TitleTransfer` is never in a fence template** (D-E6-10), so title and any `Disputed` encumbrance
survive settlement. A consignment adds `After { condition: <deposit>, delay, clock: RealUtc }` with
the delay clamped to 24–72 h (`SrEconomyConfig::fence_delay`) and **shown in the offer**.
`+fence/list` renders an org through `render_org_listed` and **omits** a hidden fence's listings for
a non-member through `may_list_org`. Notices are the contract's own — custody, settlement,
cancellation, dispute — and the ruleset adds none (answer 15). **No tick was registered**:
`store.rs`'s `TICK_TAG`/`TICK_PERIOD_SECS`/`MAX_RUNS_PER_FIRE` are still unused, because a
consignment matures on its contract's own `After`, swept by the economy, and nothing else about a
quote needs a clock the sweep does not already turn.

**B10 — the augmentation record.** `SrProfile.augmentations: Vec<InstalledAugmentation>`
(`record.rs:118`, `#[serde(default)]`, capped at `MAX_AUGMENTATIONS` 32 on load), each
`{ object, catalog, grade, essence_cost, clinic, acquisition, installed_at }` behind a `TryFrom` wire
that bounds every label and refuses an Essence cost outside `schema.rs`'s own 0..=600.
`SrProfile::install` (`record.rs:330`) records one — and (independent verify fold, M11) it has **no
production caller**: the record type ships without a producer, so B10's "settlement writes the
augmentation record" is design, not code, until the clinics slice wires it (E6c/E7).
**Payment writes the record and never the sheet**: `schema.rs`'s free-text `augmentations` map and `derive::essence_from_augmentations` are
untouched, `+karma/spend`'s job flow is untouched, and no karma↔nuyen path is added. The atomic
installation that revalidates Essence, prerequisites, custody and clinic capability is the later
clinics slice (D-E6-16).

**Two more economy additions, both game-neutral.** `deposit_into_vault(ctx, object, vault)`
(`commands.rs:930`) — the sixth programmatic seam, `@vault/deposit` without the command, so the
fence hands custody over on the seller's own standing rather than dispatching a verb from a handler.
And `parse_catalog` + `@asset/title … catalog=<ruleset>:<code>` (`commands.rs:2900`): `CatalogRef`
was **set nowhere** before this — the type existed and no producer filled it — which made every
catalogue-keyed rule unreachable. It is one option on the existing spec; the economy still never
interprets it.

**The seal, end to end, and which authority each step uses.**

| step | verb | authority |
|---|---|---|
| confer | `@scene/facilitate <scene>=<who>` | **staff** (`Role::Royalty`), and only onto an `ObjKind::Player` (08i rule 2) |
| seal | `+scene/outcome <row>=<subjects> run=<id>` | the **conferred sealer** (or staff), once, on a scene that has one at all |
| record | `mush_canon::commit` | the producer's — `mush-scenes` decides who may seal, and pins its deferred batch |
| match | `canon_set_accepts` + `names_a_party` | the **consent surface's**: kind + source + party intersection, and nothing else |
| settle | the economy's observer, else the sweep | `ContractAuth` |

**What the economy does and does not check.** It checks that the record's `kind` is a
producer-declared outcome kind, that its `source` is exactly the one the frozen condition names, and
that its `entities` name (or act for) one of the contract's parties. It does **not** check
`sealed_by`, and cannot: `WorldEvent::CanonRecorded` carries `{canon_id, kind, entities, source}` and
no `detail`. Who may seal is therefore entirely `mush-scenes`'s gate, which is why that gate is
staff-conferred and fails closed.

**Deviations, with reasons.**

1. **`+scene`'s `ArgShape` changed from `None` to `EqPair`.** It had no switches and no arguments;
   `+scene/outcome` needs both. `CommandSpec::name` is unchanged, so the rule-1 tripwire (which pins
   names) is unaffected and no stanza was needed for `+scene`.
2. **`@asset/title` gained `catalog=`, and `parse_catalog` with it.** OPEN 1 keys the fence baseline
   on `CatalogRef.code`, and `CatalogRef` was set by no producer at all — the fence would have been
   unreachable code. One option on an existing spec, no new verb, and the economy still treats the
   reference as opaque.
3. **`+fence/accept` does not accept for the seller.** 08f §4.3 says the fence accepts *first*; the
   seller then binds with `@contract/accept`, which is what makes D-E6-9's refusal reachable rather
   than theoretical — the contract really is `Offered`, with the fence's escrow standing, while the
   goods are still in the seller's hands.
4. **`WorldAuth::authorize` admits an organization acting on its own scoped compartment**
   (`api.rs:374`). An org's account is always scoped (`@account/open org:X` opens it on the root
   compartment), so DL-11's "the escrow posts under the org account's own debit authority" was
   otherwise unreachable. No command can construct a plan with an `Organization` actor — every
   command's actor is `PrincipalId::Object(ctx.actor)` — so the only producer is
   `accept_contract_as`, itself gated on the wiring-frozen table.
5. **`WorldAuth::authorize_vault` admits a `Deposit` a live contract's own `DepositInto` names**
   (`api.rs:503`). `DepositInto` was undeliverable wherever the vault was not the depositor's own: a
   seller cannot hold `vault.deposit` on a fence's compartment and never could. The contract is the
   consent (D-E4-8's rule applied to custody), the window is exactly the custody lock's — so it is
   closed before the deal is in force and closed again the moment it ends — and it admits a deposit
   only, never a withdrawal.
6. **`@sr/fence` refuses an organization the wiring did not name.** DL-11 is composition data on
   purpose, so a fence registered outside `[shadowrun.economy].vendor_orgs` could never accept
   anything. The refusal names the exact `mushd.toml` line to add (08a's staff-scarce rule: say the
   command, never "ask someone").
7. **`run:self`.** A canon-routed run must name its own contract id in a condition that freezes
   before the id exists. `EconomyStore::peek_next_contract` is a read seam and the world is
   single-threaded, so the substitution is exact; the worst a mismatch could do is leave a condition
   nothing matches, never make the wrong one match.
8. **`ruleset-shadowrun` gained no crate edge.** §2 said it "gains canon with E6b"; the producer is
   `mush-scenes` and the fence commits none, so `ruleset-shadowrun → {economy, org, party, faction,
   ruleset, dice, core, jobs, proto}` is unchanged. `mush-canon` and `mush-scenes` are
   **dev-dependencies only**, for the acceptance rig that drives the real three-crate loop.
9. **A returned deposit's last journaled custody movement stays a withdrawal.** DL-03 lands the goods
   inside the depositor's own vault backing, and the journal records exactly that: a `Withdraw` from
   the counterparty's vault to that object. `last_custody` is therefore not a `Deposit` into the home
   vault, so a *later* contract naming that vault would not see the object as deposited until its
   owner deposits it deliberately. Nothing is stranded — `holds()` asks the provider, so
   `@vault/withdraw` finds it — and the alternative (a projection rule that reads the destination's
   vault at fold time) would have made boot's replay depend on documents that load after it.

**Tests (67).** `mush-scenes/tests/e6b_seal.rs` 22, `mush-server/tests/e6b_seal.rs` 7 (the real
three-crate loop — `mush-economy` still has no `mush-canon` edge), `ruleset-shadowrun/tests/e6b.rs`
27 plus 3 in-crate wire units, `mush-economy/tests/e4b2.rs` +5, `mush-canon` +3 in-crate. The gate
(`mush-scenes`, `mush-canon`, `mush-economy`, `mush-server`, `ruleset-shadowrun`,
`mush-contract-tests`, `mush-config`) runs **996 passing, 0 failing, 2 ignored**; the workspace runs
**3 203 passing across 197 suites, 0 failing, 49 ignored**; clippy `-D warnings` and
`cargo fmt --all --check` are clean.

**Verify fold (2026-09-05).** A probe-confirming adversarial verify over three lenses — D disclosure
and bounds, M money and consent, R durability and replay — confirmed twenty-seven findings. All
twenty-seven are folded in; nineteen carry a regression test named `vf_<lens><n>_…`, and the table
says why for the rest. Four of them were **money-safety defects**, not polish: M1, M2, M3 and M4.

| item | what it was, and what changed | test |
|---|---|---|
| **M1** | A `DepositInto` condition stood in for the vault owner's consent (`WorldAuth::authorize_vault`) and was not consent at all: anyone holding `vault.view` on a compartment could name that vault in a **one-party** contract, deposit into it, and leave the object **custody-locked** where its owner could not take it out — a permanent wedge, repeatable until the vault was full. `check_offerable` now requires the vault's owner to be a party, exactly as `Deliverable::Collateral` already required of a pledger | `vf_m1_a_deposit_condition_names_a_vault_its_owner_is_a_party_to` (e4b2) |
| **M2 / R12** | `+scene/outcome`'s `run=` was **free input**: a sealer conferred on any scene could name any contract id, pick that contract's own parties as subjects, and settle a run they had nothing to do with — kind, source and party intersection all pass on that. Staff now confer **which run** a scene may seal (`@scene/facilitate <scene>=<who> [run=<id>]` → `SceneRecord.seal_run`), and the seal may name no other. A scene bound to none seals under a `scene:{uid}` source, which no contract can cite | `vf_m2_a_seal_names_only_the_run_it_was_bound_to`, `vf_m2_an_unbound_scene_seals_for_the_chronicle_and_settles_nothing` |
| **M3** | `Encumber` was staff-only before E6b, so no *party* could claim an asset; relaxing it for the title owner handed them a settlement veto — accept the escrow, file a **covert** report, and the `TransferTitle` refuses with `Encumbered` while the buyer cannot see why. `ReportStolen` is now refused on an object a **live** contract's `TitleTransfer`/`Collateral` names | `vf_m3_a_live_deals_asset_cannot_be_reported_stolen_out_from_under_it` (e4b2) |
| **M4** | DL-11's `authorize` arm sat *inside* the scoped branch, so an **unscoped** org-owned account fell through to `*actor == account.owner` — a check written when no `Organization` could ever be an actor, with no liveness test at all. The arm moved above it and returns `false` for the unscoped case; the account's own `ApprovalRule` was never bypassed and still binds | covered by `e6b_b4_the_fence_accepts_with_no_officer_and_a_broke_one_cannot` (the reachable half); the unscoped case is unreachable from any command by construction |
| **M5** | A canon source named the **run** and not the row, so `(kind, source)` — all the economy has, since `CanonRecorded` carries no `detail` — made one seal fill *every* canon row's slot (`Undecidable`, dispute), and on a single-row table the row name the sealer typed decided nothing at all. The source is `run:{id}#{row}` on both sides | `vf_m5_a_canon_route_names_its_row_so_the_seal_selects_it` (e6b), `vf_m5_the_seals_source_names_the_row_it_answers` (e6b_seal) |
| **M6** | `claimant == actor` was an escape hatch: a person holding `asset.insure` on an org's compartment could record a `Disputed` claim in **their own** name on the org's asset. A theft report is a claim about a *title*, so it names that title's owner and nobody else | covered by `e6b_b2_the_owner_an_insurer_and_staff_may_file_and_a_bystander_may_not`; `@asset/stolen` derives the claimant, so the hatch was reachable only through the `pub` plan seam |
| **M7 / R6** | `stranded_deposits` asked a *second* predicate and the two disagreed: a deposit whose object had left the vault, or whose home vault's backing was destroyed, was dropped by `returnable` and reported by nothing. It is now the complement of `returnable`, and the page names the object | covered by `e6b_b3_a_depositor_org_with_no_vault_is_stranded_not_silently_dropped`; the divergent shapes need a destroyed backing mid-unwind, which no shipped verb sequences |
| **M11** | A `canon:` route's source was free-hand, so one listing could piggyback on another's seal. `+run/post` now refuses a `run:<n>` source that is not this run's | `vf_m11_a_canon_route_naming_another_run_is_refused_at_post` |
| **M12** | `run_custody_effects` chose the mover as "the plan actor, else the destination", but `refund_plan` names the *first* return's depositor — so a contract with one object and one org depositor handed that person's identity to the `CarryAttempt` gate for the org's goods. The backing is the mover **per effect**, for a home return only | preventive; `authorize_vault_home` already constrained the destination, so no laundering was reachable |
| **D1** | `AssetOp::ReportStolen` bumps `EconomicAsset.revision` like every asset op, and a quote was bound to that revision — so filing a **covert** report flipped the holder's outstanding quote from `open` to `lapsed` where they could read it. The price was identical in both worlds and the *rendered state* was not. `QuoteObserved.declared` is now a hash of the declared catalogue code | `vf_d1_a_covert_report_does_not_lapse_the_holders_quote` |
| **D2** | `encumbrances: {n}` sat one line above the gated report, so an assignee read a covert filing off the arithmetic. A claim the viewer may not be told about is not counted for them | `vf_d2_the_encumbrance_count_does_not_disclose_a_covert_claim` |
| **D3** | `+fence/list` paged **before** it filtered, so a member's and a non-member's page truncated differently. Filter, then page — and the walk is bounded on records *visited* (`FENCE_PAGE_VISIT`) | `vf_d3_the_fence_page_filters_before_it_pages` |
| **D4** | The vendor exemption from `consideration_for`'s view gate widened the predicate for *every* consideration on *every* offer, so the next caller of that `pub` seam would have inherited an existence oracle. It is now narrowed to a vendor that is a party to **this** contract | preventive; both shipped callers pre-gate, so nothing player-typed reached it |
| **D5** | `rescan_canon_sources` bounded the sources it *matched* and not the contracts it *visited*, so on a world where few contracts carry a sourced `Canon` set — the normal case — it walked every contract on every activation. `MAX_CANON_CONTRACTS_VISITED` (D-LBK-5's rule) | preventive; the bound is a `take`, and a counter on the path would be production code existing only for a test (E6a fold deviation 3) |
| **D6** | `MAX_FENCES` and the quote ceiling were enforced on write only, so a store carrying more booted with more. Both loaders now stop at the cap and log the skip, against a new whole-store `MAX_QUOTES` | `vf_d6_the_quote_cap_is_enforced_on_load` |
| **D7** | The engine's `AlreadyClaimed` told an `asset.insure` holder that a **covert** report stood whether or not `may_view_stolen_report` said they could see it — so the disclosure happened anyway, as an error message. Covert means invisible to the **thief**, not to the insurer whose claim it is, so the insurer is now inside it | covered by `e6b_b2_a_covert_report_is_invisible_to_the_thief` |
| **D8** | C16's tally exists so staff can see the shape of an id-space walk, and `FenceId`/`QuoteId` are two brand-new enumerable ones — a refusal built straight into a `CmdError` never reached it. `refuse_fence`, `refuse_board` and the `+fence` id parses all count now | `vf_d8_a_fence_id_walk_is_counted_by_the_refusal_telemetry` |
| **D9** | `+scene/outcome` gave two different refusals for "nobody was conferred the seal" and "somebody was, and it is not you", and conferral is told only to the staffer who made it — so any participant could read a conferral off the difference. One message; staff keep the detail | `vf_d9_the_seal_refusal_does_not_disclose_whether_one_was_conferred` |
| **D10** | Both quote reads collected the whole match set before paging. Bounded on records visited, newest first | covered by `vf_d3`'s shape; the same `FENCE_PAGE_VISIT` bound |
| **D11** | `owner_home` scanned every vault, three times per unwind. `MAX_HOME_VAULT_VISIT` | preventive |
| **R1** | `record.rs` said clearing a theft report was `@asset/release` of its `Disputed` claim, and it was not: nothing anywhere wrote `stolen = None`, so a recovered asset carried the report for ever and refused a **genuine second theft**. The claim and the report now end as one act | `vf_r1_releasing_the_claim_clears_the_report_so_a_second_theft_can_be_filed` |
| **R2** | A retcon appends rather than rewrites and emits no `CanonRecorded`, so the economy never hears it — and DL-09's recovery would have resurrected a seal a staffer had already superseded, settling a contract on retracted evidence the live path could never have used. `records_for_source` drops superseded ids | `vf_r2_a_retconned_record_is_not_returned_as_evidence` |
| **R3** | The three new `SceneRecord` fields are `#[serde(default)]`, so **forward** no migration is needed — that half of OPEN 5 is confirmed. The other direction was open: a rollback to the pre-E6b binary booted happily on a v2 namespace and its first write-through would erase a *terminal* sealed outcome the economy had already cited. A v3 identity migration is the stamp that makes the runner refuse | `vf_r3_the_namespace_is_stamped_so_a_rollback_refuses` |
| **R4** | `@scene/icfloor` is monotonic and has **no inverse anywhere**, so one pasted epoch-millis value would stamp every future record from that scene at the year 55 000 for ever. Bounded to `MAX_IC_FLOOR_LEAD_SECS` ahead of the world's own IC clock, and refused once the outcome is canon | `vf_r4_the_ic_floor_cannot_be_raised_past_the_world_or_after_the_seal` |
| **R5** | The source re-scan remembered every record it found, and one re-scan can return 32 × 8 — so it replaced the 64-deep tail with exactly the records that were already recoverable, evicting the ones that were not. It remembers none of them | preventive; the eviction is invisible until a contract offered later cites an evicted source |
| **R8** | The allocator floors came from what **loaded**, so a corrupt `sr:fence:{n}` plus a lost `sr:meta` re-minted id `n` and wrote straight over the evidence. They come from every document's parsed **key** now, loaded or not — `sr:run:` included | `vf_r8_a_corrupt_document_still_claims_its_id` |
| **R9** | `mush_canon::commit` applied no cap to `CanonDraft.entities` while the economy's tail truncates at 64, so a wide record matched on the live edge and was silently missed by the recovery. `commit` caps at `MAX_ENTITIES_PER_EVENT`, the same number | `vf_r9_the_commit_cap_covers_every_producers_widest_record` |
| **R10** | `stolen` is journal authority but was absent from `journal_fields_eq`, which is *also* the "re-persist this doc" predicate — so a document whose report alone disagreed with the journal was neither frozen nor healed. (The freeze concern was **refuted**: `put_asset` adopts the journal value, so replay equals live and no boot could freeze on it) | preventive; the comparator is private and the fail direction was already safe |
| **R11** | The re-scan settles, a settlement emits, and an observer could in principle activate a contract and re-enter it. No shipped observer does — but "no caller exists" is not a bound. A `canon_rescanning` latch is | preventive |

**Refuted while verifying** (recorded because a refuted claim is worth as much as a found one): the
fence's pricing really does read only declared inputs — `price` and `quotable_code` touch the
catalogue code and nothing else, and D1's leak was through the *revision stamp*, never the price;
`+scene/outcome`'s `mush_canon::commit` really is the last mutating act on every path that can still
fail, with every fallible step before the first mutation; `authorize_vault_home` is not forgeable and
`@vault/withdraw` cannot reach it (`WorldAuth` keeps the `false` default and the command posts under
`PostAuth::World`); `accept_contract_as` is bounded three ways over and no command can construct an
`Organization`-actor plan; the plan-hash pins are untouched, because `AssetOp` serializes by variant
**name** and ordinals 0–7 did not move; the once-per-boot re-scan really is once, and the
party-intersection gate really does run on both recovery paths; the fence's `by_org` /
`by_quote_contract` projections are re-derived in exactly four places and cannot go stale; and the
two new document families' loads are genuinely total — a bad key, a bad shape and an id/key mismatch
are all a `tracing::warn!` and a skip.

**Independent verify fold (2026-09-05).** A second, **independent** adversarial verify over the same
three lenses — D disclosure and bounds, R durability and replay, M money and consent — probe-confirmed
twenty-eight findings the builder's own fold above had missed. All twenty-eight are folded in;
twenty-four carry a regression test named `vf2_<lens><n>_…`, two are doc-only (M10, M11), and the
table says why for the rest. **Four were money-safety defects** — M1, M2, M4 and M5 — and **four were
recovery defects that failed silently** — R1, R2, R3 and R4.

| item | what it was, and what changed | test |
|---|---|---|
| **M1** | **One object, one delivery, N payments.** `TypedCondition::DepositInto` is a pure state predicate and nothing asked whether another live contract was already being paid for the same delivery — so eight quotes on one item were eight escrows, all satisfied by the single `+fence/deliver` that satisfies the first, draining the fence by arithmetic. Both layers: `fence::quote` refuses an object that already carries an open (current or contracted) quote **by any seller**, and — game-neutral, the `MAX_DEPOSITS` 1 spirit applied across contracts — `check_offerable` and `activate` refuse a `DepositInto` a live contract already names (`snapshot.deposit_lock_of`, an O(log n) range over the existing lock index). `activate` is the last gate because two offers can both pass the first | `vf2_m1_one_object_is_one_open_quote_and_one_live_deposit` (e6b) |
| **M2** | The payer settled a canon-routed run by attesting the **unrouted** row. `parse_routes` defaults every unnamed row to `attest`, B8 deliberately names no person adjudicator on a routed run, so `consent.adjudicator` was `None` and `contract_attest`'s `None` arm handed the payer their own `Party(payer)` set — while `AdjudicatorShape::Impersonal` told the crew check there was no conflict to find. Once any row is routed, either every row is, or `adjudicator=` is named | `vf2_m2_a_partly_routed_run_must_name_the_adjudicator_of_the_rest` |
| **M3** | `gate=canon:<kind>:run:self` could never be satisfied: the gate's `run:self` resolves to a bare `run:{next}` and the producer's seal always emits `run:{contract}#{row}` (M5's own fix). A crewed run carrying one froze its escrow to the deadline with nothing saying why. `parse_gates` refuses a `canon:` gate — canon evidence is per row, and a gate is not a row | `vf2_m3_a_canon_gate_is_refused_at_post` |
| **M4** | **A run's own payee could be conferred the seal that pays them.** `@scene/facilitate` performed no party check at all, and the shipped acceptance test *was* the exploit. `mush-scenes` gains a composition-frozen `SealerGate` (`Arc<dyn Fn(&World, u64, ObjId) -> bool>`, `ScenesModule::with_sealer_gate`), wired by the server from the economy's new `stands_in_contract` (party through `acts_for`, or author / payer / Johnson / facilitator). Asked at conferral **and** again at seal, because a roster moves in between. `mush-scenes → {core, canon, proto}` is unchanged — the run id stays opaque there — and the acceptance rig now seals with a non-party | `vf2_m4_a_runs_own_payee_cannot_be_conferred_the_seal_that_pays_them`, `vf2_m4_the_gate_is_asked_again_at_seal_because_the_roster_moves` |
| **M5** | A seller locked a fence's liquidity for goods they do not hold: `fence::accept` re-checked `is_current` (expiry + revisions) but never custody, so a quoted seller could hand the goods away and still make the fence escrow the full price against a deal `+fence/deliver` can never complete. `accept` re-asserts the object's location is the seller — the same world fact `quote` asks, asked again where the money moves | `vf2_m5_accept_re_asserts_custody_before_the_fence_escrows` |
| **M6** | `stranded_deposits` filtered on `r.to.is_none()` alone while `returnable` asked a second question, so a deposit with a destination that could not be used was dropped by the plan and reported by nothing. It is the complement of `returnable`, and the page names the object | `vf2_m6_a_deposit_whose_object_left_the_vault_is_paged_not_dropped` |
| **M7** | `CatalogRef` was writable only by `AssetOp::Title`, and `Title` refuses an object with a live record — so **every title minted before E6b could never be catalogued**, no pre-E6b asset could ever be fenced, and D-E6-13's stale-revision refusal was near-unreachable. `AssetOp::SetCatalog` at **ordinal 9** (nothing renumbered; the two digest pins stay green because `AssetOp` serializes by variant name), staff authority, and `@asset/catalog <obj>=<ruleset>:<code>|none` as a switch on the existing `@asset` spec. It bumps the asset's revision like every other op, which is what makes a re-catalogue lapse an outstanding quote deterministically | `vf2_m7_an_existing_title_can_be_catalogued_and_the_change_lapses_a_quote` |
| **M8** | `condition_holds` **ORed** the condition's own attestor set with the adjudicator's, where `@contract/attest` and the observer's `answers` both **replace** — so an attestation neither writing path would record still re-verified, settling a contract whose parties consented to one authority and got another. Replace, in all three (D-E6-8: one rule) | `vf2_m8_reverification_replaces_the_condition_set_with_the_adjudicators` |
| **M9** | Re-registering a fence's vault mid-deal silently killed every live deal: a frozen `DepositInto` names the old vault by id and no re-pointing can re-open a frozen surface. `@sr/fence` refuses to change the vault while the fence has current quotes or live contracts, and says what has to end first; name and demand stay editable | `vf2_m9_a_fences_vault_does_not_move_under_a_live_deal` |
| **M10** (doc) | The economy compares a canon `source` as an **opaque byte string** and interprets nothing about it, so a hand-built `@contract/condition canon:<kind>:<source>` accepts any source by design. The allowlist that keeps a run board's route on its own run is therefore the **producer's** (D1), never this crate's. Stated in `attest.rs`'s module doc and here | none (doc) |
| **M11** (doc) | B10's "settlement writes the augmentation record" is **not built**: `SrProfile::install` has no production caller, so the record type ships without a producer. The clinics slice wires it (handoff to E6c/E7) | none (doc) |
| **D1** | The `canon:` source check was a **denylist** — it refused only a `run:<n>` naming another run — so every other prefix listed: a run could cite `scene:{uid}`, which is exactly what a scene bound to no run seals under, making staff's "chronicle only" conferral a live settlement authority. Inverted to an **allowlist**: after the `run:self` substitution a canon source is exactly `run:{next}` or `run:{next}#{one of this run's own rows}`. This also bounds the source (D6) | `vf2_d1_a_canon_route_is_allowlisted_to_this_runs_own_rows` |
| **D2** | `@asset/release <obj>=<n>` was an index oracle: out of range answered "No such encumbrance.", in range answered "unauthorized", and an assignee holds `may_view_asset` — so a covert theft report was readable off the difference. An index resolving to a hidden report's own `Disputed` claim answers exactly as an index past the end does | `vf2_d2_the_release_index_is_not_an_oracle_for_a_covert_report` |
| **D3** | The `scene.outcome` chronicle line named the contract to a **non-party**: the record is `Restricted` and `+chronicle` shows it to every entity it names, but the entities are the seal's *subjects*, who need not be parties. `{source}` is out of `summary`; it stays in `detail` and in `CanonRecorded`, where it is load-bearing | `vf2_d3_the_chronicle_summary_does_not_name_the_contract` |
| **D4** | A lapsed quote was **permanent** and wedged its seller: `close_quote` runs only from the contract observer, so an expired quote was never deleted and the per-seller count included it — eight lapses locked a seller out of the fence for ever, behind a refusal telling them to let one lapse. The count is of live quotes (`is_current(q) || q.contract.is_some()`), the walk deletes the dead ones it passes (`MAX_LAPSED_SWEEP_PER_CALL` 32), and two new store indexes (`quotes_of_seller`, `quotes_for_object`) make both walks the seller's own rather than the registry's | `vf2_d4_lapsed_quotes_are_swept_and_do_not_wedge_their_seller` |
| **D5** | `MAX_QUOTES` was enforced **on load only** while `quote()` checked the per-seller eight, so many sellers put the store past the ceiling legitimately and the next boot silently dropped the overflow — contract-bound quotes included. The same refusal on write ("The fences are busy; try later."), and at the cap the loader lets a **contract-bound** quote displace an unbound one, because a bound quote is a handle on a standing escrow rather than a re-askable price | `vf2_d5_the_quote_ceiling_is_enforced_on_write_and_the_loader_keeps_the_bound_ones` |
| **D6** | A `Canon` set's `kind` and `source` had **no length cap anywhere** — `parse_route_spec` takes the source as the remainder verbatim — and both are frozen into the consent surface and rendered to every party. `check_structure_basic` caps both at `MAX_OUTCOME_NAME_BYTES`, game-neutrally, so no producer of a `ContractSpec` can freeze an oversized set | `vf2_d6_an_oversized_canon_source_is_refused_at_offer` |
| **D7** | `@econ/history` collapses every masked **line** into one `MASKED_ACCOUNTS` token precisely so a runner cannot count the other runners, and then published the same arity through the hold half, one line per hidden hold. One token, the same way | `vf2_d7_hidden_holds_collapse_into_one_line` |
| **D8** | `EconomicAsset` was raw `Deserialize`, so `StolenReport.reason`, a `Disputed` claim's reason and the claim count were bounded on the write path only. `EconomicAssetWire` is the `TryFrom` carrying those caps, and boot **skips** an asset document that fails them loudly instead of refusing to start: every field above the latch is journal authority, so the projection rebuilds from the journal and comes back `frozen` with reason `doc-lost` — exactly what a vanished document already gets | `vf2_d8_an_oversized_theft_report_reason_does_not_load` |
| **D9** | `+scene`'s allowlist stanza was still `baseline` — pinned when it was `ArgShape::None` with no switches, i.e. a pure read — though `+scene/outcome` now writes a terminal sealed outcome and commits canon an escrow settles from. It is `legacy-post-image` with `family = "scenes:scene"` and its reasons; `PINNED_BASELINE` 159 → 158, and the tripwire still fails on an unlisted verb | the rule-1 tripwire itself |
| **D10** | `@asset/list` walked every asset before the view gate. Bounded on records **visited** (`ASSET_PAGE_VISIT` 256) with an `after=` cursor, `+fence/list`'s shape; the owner filter reads the `assets_of` index; and the cursor is the last record *visited*, so a narrow viewer and a staffer walk the same pages | `vf2_d10_asset_list_is_bounded_on_records_visited_and_pages` |
| **R1** | `rescan_canon_sources` took its 256-contract budget **before** filtering live, and `contracts` is ascending by id with terminal records never collected — so on any world with 256 dead contracts the whole budget went on them and the live run was never visited: DL-09's recovery stopped **silently**. Filter, then take. That is the rule for every bounded walk in this cut, and R2 is the same defect on `owner_home` | `vf2_r1_the_source_rescan_filters_live_before_it_takes_its_budget` |
| **R2** | `owner_home` took 256 vaults before filtering owner and `Active`, so an org whose vault sorted past the bound resolved `None`: the deposit stayed in the counterparty's vault and the stranded page's own remedy (`@vault/create`) minted a **higher** id — never clearable. `EconomySnapshot` gains `vaults_by_owner` / `vaults_of` beside `accounts_of`, and the walk filters first | `vf2_r2_owner_home_filters_by_owner_before_it_takes_its_budget` |
| **R3** | The durable canon read seam was **empty whenever the tail chunk sealed**: `records_for_source` reads the ledger's open tail, and `Ledger::restore_from_kv` restores a sealed tail as an *empty* open chunk — so a seal committed just before a 16 KB boundary was unrecoverable across the very reboot DL-09 exists for. Two parts: `mush_core::recent_sealed(kv, log, n)` decodes the trailing sealed chunks, and `CanonModule` keeps the last `SEALED_TAIL_CHUNKS` 2 in `CanonState.sealed_tail` (decoded once, at boot, from documents it already holds) for `records_for_source` to read beside the open tail; and a source the seam answers with **nothing** now pages the ladder on the contract's subject (`SettlementRefused`, `@contract/resolve <id>=settle`) instead of returning silently — from the recovery triggers only, so a freshly offered run is not a stuck one | `vf2_r3_a_seal_in_a_sealed_chunk_is_still_found_across_the_reboot`, `vf2_r3_a_source_no_record_answers_pages_the_ladder` |
| **R4** | **A pre-amendment seal re-attested the amended surface.** `amended_draft` clears `satisfied`, but a canon source names the run and the row and no *generation* — so the next re-scan found the same record and put the same attestation straight back: a seal made before the crew joined would have paid the crew. `EconomicContract.generation_started_at` (`#[serde(default)]`) is stamped at draft and at every amendment, `CanonRecord` carries the record's own `at`, and `record_and_settle` skips (and logs) evidence older than the generation it would attest. The producer cannot know the generation, so the check is the economy's | `vf2_r4_a_pre_amendment_seal_does_not_re_attest_the_amended_surface` |
| **R5** | `rescan_canon_tail` walked the 64-deep RAM tail **before** the `canon_rescanning` latch was taken — the latch lived one level down, in the source re-scan. Moved to the entry point; boot's direct `rescan_canon_sources` call takes it too, and the shared body is `rescan_sources_inner` | `vf2_r5_the_rescan_latch_covers_the_tail_walk_too` |
| **R6** | A corrupt `SceneOutcome` **bricked the boot**: its `TryFrom` failure failed the whole `SceneRecord`, which `load_boot` turned into a `ModError`, so one over-long note was a server that could not start. The outcome is decoded leniently at boot (`strip_unreadable_outcome`): the scene loads with `outcome: None`, keeps its `sealer` and `seal_run`, and the loss is loud. The economy's write-once slots make a re-seal harmless, so "seal it again" is a real remedy. Every other field stays strict | `vf2_r6_a_corrupt_persisted_outcome_does_not_brick_the_boot` |
| **R7** | `records_for_source` decoded the open tail **twice** per lookup (`superseded_in_tail()` + `recent()`), and the re-scan asks it per source. Decoded once, with `CanonState::superseded_in(&[CanonEvent])` over the slice | none (no behaviour to regress; covered by R3's tests exercising the same path) |

**Deviations from the fold brief, with reasons.**

1. **R3's page fires only from the recovery triggers** (boot's first sweep and `@econ/unfreeze`), never from a contract's own activation. A canon-routed run that was offered five minutes ago and whose scene has not been played yet has no evidence either, and paging staff about it would be exactly the "wait for staff" parking 08a's staff-scarce rule forbids. The ladder's per-subject dedupe then makes it one page per stuck contract however often the re-scan runs.
2. **R6 logs loudly and does not page.** `mush-scenes` has no economy edge and the escalation ladder is the economy's; adding one to page with would be a crate edge bought for a log line. The `tracing::error!` names the scene and says the sealer may seal it again, which is the whole remedy.
3. **M7's `SetCatalog` is staff-only in `WorldAuth`**, not "staff or title issuer". The title-issuer seam builds its own `Title` plan and declares the catalogue there, so an issuer arm would be unreachable code — the same reason E6a's fold deviation 3 gives for not adding production code a test alone would exercise.
4. **`e4c.rs`'s `vf_d4` assertion was tightened, not changed.** It counted the substring `other accounts`, and D7's hold token contains it; it now counts whole comma-separated parts, which is a strictly more precise statement of the same property.
5. **Four shipped `e6b.rs` tests were amended to satisfy M2 and M1**, not weakened: three now name an adjudicator or route every row (which is what M2 says a run must do), and the expired-quote test no longer rewinds the clock, because M1 refuses a second quote on an object whose first is live again.

**Handoffs to E6c.** `with_party_caps` and `accept_contract_as` are the vendor accept E6c's market
reuses for vendors and makers (G8), and `render_org_listed`/`may_list_org` are the pair a hidden
vendor's listings are omitted through (G3). `CatalogRef` now has a producer
(`@asset/title … catalog=`) and a consumer (the fence baseline), so E6c's catalogue replaces the
source-controlled `fence_baseline` table rather than inventing the first one. `quote_ttl_live` /
`quote_ttl_remote` / `catalog_paths` / `chargen_carryover_cap` are still parsed and inert. The
`AcquisitionRecord` a fence settlement stamps is the provenance seed doc 08 §9.4's lineage grows
from. And the residual E7 owes: a DL-03 home return's last journaled custody movement is the
withdrawal it really was, so a *later* contract naming the destination vault does not see the object
as deposited there until its owner deposits it deliberately — `holds()` asks the provider, so nothing
is stranded, and the alternative (a projection rule reading the destination's vault at fold time)
would make boot's replay depend on documents that load after it.

---

## 5. E6c — the market

### 5.1 Catalog and representation policy (answers 9–10, doc 10 T5)

```rust
pub struct CatalogEntry {                 // `market:catalog:{pack}:{code}`
    pub catalog: CatalogRef,              // { ruleset, code } — the economy's opaque precursor (record.rs:556-562)
    pub pack: SmolStr,
    pub definition_version: u32,          // bumped by the pack; a quote records the version it priced
    pub name: SmolStr,
    pub baseline: Option<Amount>,         // a REFERENCE price, never an intrinsic one (doc 08 §9.4 / D9)
    pub availability: AvailabilityRule,   // None | Test { skill, threshold, interval } | Attested
    pub legality: Legality,               // Legal | Restricted | Forbidden
    pub representation: Representation,   // THE POLICY DECISION
    pub stackable: bool,  pub tags: BTreeSet<SmolStr>,
    pub rules_ref: Option<SmolStr>,       // a book/page citation, never mechanics
}
pub enum Representation {
    Instrument,   // fungible commodity: an InstrumentKind::Commodity account (record.rs:52-61)
    Lot,          // provenance-bearing: a Lot with immutable lineage (doc 10 T5)
    Asset,        // an EconomicAsset with title, custody, encumbrances (E3)
    Entitlement,  // InstrumentKind::Entitlement — licences, programs; non-transferable
    Installed,    // becomes an InstalledAugmentation/component on installation (§4.4)
}
pub struct Lot {                          // `market:lot:{id}`
    pub id: LotId, pub catalog: CatalogRef, pub owner: PrincipalId,
    pub quantity: u64, pub grade: u8, pub legal: Legality,
    pub parents: Vec<LotId>,              // split → one parent; merge → all. NEVER discarded (T5)
    pub created_at: i64, pub revision: u64,
}
```

The policy is decided **per entry by category and gameplay significance, with price only as a hint**
(answer 9): important, serialized, restricted, upgradeable or stealable gear is `Asset`; ammunition,
drugs, reagents and commodities are `Instrument`, or `Lot` where grade, source or legal status
matters (T5); licences and programs are `Entitlement`; cyberware is `Installed`. The starter pack is
small and representative — weapons, armour, drugs, programs, a deck, magical goods, a service,
cyberware — never an exhaustive transcription (answer 10), and vendors produce offers *from* the
catalog without mutating an entry. Split and merge write child lots naming their parents; **lineage
is append-only and never discarded**, though a rendered view may coarsen it (T5). Coarsening rules
and legal-status propagation beyond the starter set are E7's (doc 10 §16 item 2).

### 5.2 Listings, quotes, vendors

```rust
pub struct Listing { pub id, pub seller: PrincipalId, pub catalog: CatalogRef, pub source: Supply,
                     pub price: Amount, pub quantity: u64, pub visibility: MarketVisibility,
                     pub expires: EconomicDeadline, pub revision: u64 }
pub enum Supply { Stock { vault: VaultId },                 // held in the storefront vault
                  Market { source: MarketSourceId },        // a registered NPC market source
                  Procurement { lead: EconomicDeadline } }  // a delayed delivery contract
pub struct Quote { pub id, pub listing, pub buyer: PrincipalId, pub price: Amount,
                   pub definition_version: u32, pub observed: RevisionSet,
                   pub expires: EconomicDeadline,           // ITS CLOCK IS DECLARED (answer 11)
                   pub availability: Option<AvailabilityOutcome> }
```

**Quote expiry declares its clock** (answer 11): `QUOTE_TTL_LIVE` ≈ 1 h on `RealUtc` for an in-person
session, `QUOTE_TTL_REMOTE` ≈ 24 h for asynchronous offers, both configurable. A quote whose
`definition_version` or `observed` revisions have moved is refused **deterministically**, not
re-priced (scenario 29). Stocked legal goods need no availability test; scarce or restricted goods
run an **engine-resolved, recorded** Shadowrun test through the ruleset's own dice
(`crates/ruleset-shadowrun/src/roll.rs`), stored on the quote; ST attestation is only for exceptional
or authored availability. Accepting a quote drafts an ordinary E4b2 contract — consideration, a
`TitleTransfer` or `DepositInto` deliverable, a `Before` deadline — so E6c adds no settlement
machinery at all. **Only the issuer offers** (E4b2's rule), so a buyer
accepting a quote is the issuer and the vendor accepts programmatically through DL-11's
`with_party_caps`, landed with E6b's fence and reused here (G8); a hidden vendor renders through
`render_org_listed` and its listings are absent for non-members, never "No such" (G3). **NPC market makers (doc 10 7A):** `MarketSource { principal: OrgId, cash:
AccountId, inventory: VaultId, spread, volume_budget, loss_budget, window }`. A maker quotes only
inside its budgets and **stops quoting** when cash or inventory runs out. Funding is explicit —
world-start capitalization, sales, or a `create_system_obligation` drip from a registered faucet
under the `[economy]` mint cap — never an unlimited account (7A; answer 4's rule applied to NPCs).

### 5.3 The safe public projection (doc 10 §14)

The projection is E6c's gate and is deliberately **aggregate**: per `(catalog, market)` it publishes
last trade, a bounded price band, depth by price level, and volume over a window. It publishes **no
account id, no listing id the viewer may not see, no counterparty, and no quantity attributable to
one seller**, and it coexists with the existence-hiding renderer rather than bypassing it — a listing
the actor may not view is "No such listing.", identical for nonexistent and unviewable, and an
unauthorized filter returns an empty list (08a decision 13). Depth is bucketed so a single
distinctive quantity cannot identify its owner. That is the whole of doc 10 §14(b)'s "explicit safe
projection, not the full belief engine".

```text
+market/list [tag=|legal=]   +market/show <catalog>   +market/depth <catalog>
+market/quote <listing>[=<qty>]   +market/accept <quote>   +market/sell <obj>=<vendor>   +market/orders
@market/catalog load|list|show <pack>   @market/vendor create|fund|stock|retire <vendor>
@market/source create|budget|halt <source>                                        (all staff)
```

### 5.4 E6c tests (~28)

**Catalog:** a pack loads, versions and reloads idempotently; a bad entry fails the pack, not the server; each `Representation` produces the right artefact on purchase (a `Commodity` account line, a `Lot`, an `EconomicAsset` with title, an `Entitlement`, an `InstalledAugmentation`); a split then merge keeps every parent in the lineage and a coarsened render never loses it.
**Quotes:** expiry on each declared clock; a stale `definition_version` refuses; a stale `observed` revision refuses; an availability test is recorded and a failure refuses without charging.
**Vendors:** stock, market-source and procurement each settle through a real contract; a procurement's lead time is a `Before` deadline.
**Projection:** two worlds differing only in hidden listings publish identical projections; a single large listing is not identifiable from depth; an unviewable listing renders "No such listing."; filters return empty rather than refusing.
**Makers:** a maker stops quoting at its cash, inventory and loss budgets; funding above the mint cap is refused; a halted maker's outstanding quotes expire and settle nothing.

**Hazards on this axis.** *Market existence oracles:* aggregate-only projection, bucketed depth, the shared existence-hiding renderer. *Stale quotes:* version + revision binding, refused deterministically. *Insolvency:* explicit budgets checked before every quote, no faucet behind them.

---

## 6. Build discipline

As 08c §7 and 08a's standing rhythm, per cut: (1) confirm the cut's slice of this plan against the
tree; (2) build, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`;
(3) **a probe-confirming adversarial verify of that cut** — three lenses, throwaway probe tests —
before the next cut builds on it; (4) fold every confirmed finding with a regression test named
`vf_<lens><n>_…`; (5) commit with the verify outcome in the message; **push only on the owner's
word**. Both digest pins (E2 `9c917eee…`, E3-shaped `b780e3a0…`) stay green in every cut, and
every cut is **fmt-clean** (`rustfmt.toml`; CI's `Format` job, `cargo fmt --all --check`, blocking
since the whole-tree reformat `91208ab`) — 08a's "`cargo fmt` stays off" is superseded.

## 7. What E6 feeds back

1. **Per-party amendment binding (E4).** 08c §5.1 deviation 3's open question, answered by use: a roster substitution is the common case and voiding every acceptance to serve it is wrong. E6a ships **A′** — consent by authorship, the amending party's own acceptance survives (§1.1, §2 item 1) — which covers the payer's crewing; a substitution after the runners accepted still voids theirs. Full per-party binding (a per-party sub-hash over the surface slices that party is bound to) stays an E4 consent-model change.
2. **A real consumer for `ExpenseTreatment` (E4).** E6a uses `Retained`/`Returned` through explicit payouts; `Reimbursed` still has none until receipts exist (answer 7).
3. **Participation fact kinds (E7; doc 10 §15 R6/T13).** E6a's frozen roster is the Shadowrun analogue of `sim.station.served`: register `sr.run.participated { party, actor, from, to }` beside the T2 boundary facts, answered from an **applied marker**, never a status (08c §6.2). Doc 10 §15 also gets (G10): account-distinctness at roster lock (R3, C23/C24) — `+run/crew accept` calls the core/org boolean oracle and stores the verdict; `consent_hash` goes `pub` for T10's consent grants; `sr.run.participated` stays E7 and is answered from `Settled{transaction}`, never from board state.
4. **The `scene.outcome` record shape (E7).** Doc 10 T6's declared operations and §16 item 5's campaign layer need the same thing — an outcome-bearing, conferred-authority seal naming its subjects — and should reuse it. The **scene IC time floor** (doc 10 §15/6A) is the proper fix for §4.1's `ic_time` workaround. T10's proxy-seat invitation is the same audience-visibility contract template as an invitation-only run board entry.
5. **`report_stolen` is generic (E5/E7).** Insurance (doc 10 §14) and salvage claims consume the same `Disputed` encumbrance. **`mush-market` serves WoD and TacSim** — answer 20's whole point.
6. **`@econ/cadence` shipped in E5b** (§11 q2); E6a tests E4b1's B6/S2 against real clocks, and `sr.month` becomes a declared cadence at D-LBK-7's convergence (§9).

## 8. Decisions

- **D-E6-1 `sr.nuyen` is scale 0 and the account is authoritative after approval** (answer 1); the sheet field survives only as a chargen budget and migration source; the instrument renders through `InstrumentDisplay { symbol: "¥", position: Prefix }` (E2-C16, 08g §12.1 item 4).
- **D-E6-2 Karma never touches the economy** (answer 2): no instrument, no conversion, no deliverable.
- **D-E6-3 Lifestyle is one fixed-amount obligation on an `sr.month` cadence**, created through `create_obligation` (`cause: Ruleset{"sr.lifestyle"}`), and a downgrade is driven by a *missed* period, never a forgiven one, at most one per period (answer 3). `Obligation.amount` is a fixed `Amount`, so a tier change replaces the obligation rather than scaling it — **and so does the downgrade**: `Delinquent` leaves the due pool and autopay stops (`obligation.rs:171-173`, `sweep.rs:459-461`), so the second miss replaces the record one tier down with `unpaid_periods` carried; `+lifestyle/set` is refused in `Grace`/`Delinquent`, so a tier change can never wipe a miss; the schedule's `maximum_catch_up` is 1, so a backlog is one attempt and never a second miss (M11, D-E6-23).
- **D-E6-4 A campaign or NPC principal that funds a run is an `Organization`, not a `System` principal** — funded by `create_system_obligation` under the `[economy]` mint cap (answer 4, doc 10 7A). The tree makes a `System` payer *unfundable* (nobody acts for one, staff included — 08c §4.7's verify fold) but *offerable*: `check_structure_basic` refuses no `System` party or `from`, `parse_principal` accepts `sys:`, and the offer would stand as a zombie nobody can accept and nothing expires (M4). E6a refuses it at post — decided on the account owner before any draft exists, rendered as that account's existence-hiding text, never a faucet-shaped message (E4c D1), tested by "no contract was drafted" — and the fold refuses it in `check_structure` (§2 item 9). Faucets mint *into* org accounts; they are never contract parties.
- **D-E6-5 The party is a record, not a principal, and the contract never points at it** (answers 5, 19, 21; 08c §5.1's E6a handoff). The roster freezes *into* the consent surface as parties and named payout accounts; `Party.status = Locked { contract }` is the party-side back-reference only, set by the ruleset's own seam at `+run/crew accept` (§3.2) and released by its `EconomyObserver` on the contract's terminal `ContractChanged` — the same observer mirrors `Active` to the board's `Crewed` (the housing precedent, `mush-housing/src/econ.rs:96-108`; the wiring of §2 item 8).
- **D-E6-6 A run always carries an outcome table, even with one row** (answers 7–8) — the only way to make "no unspecified remainder to the fixer" structural, since `remainder_to` is inert whenever `outcomes` is non-empty (`contract.rs:268-270`) and each row's payouts must sum exactly.
- **D-E6-7 `scene.outcome` is sealed by a staff-conferred sealer or staff — never a self-granted facilitator — and names its run** (answer 6; E4c fold E1; §4.1 re-grounded at 0.3): `source: run:{contract id}`, because a `scene:{uid}` is unknowable at offer, and `entities = subjects ∪ {sealed_by}` named at seal time, not copied from `participants`. The named adjudicator ends the run by `@contract/attest` (D-E6-8) and is never a party or the poster (answer 22); an economy-side `sealed_by` check is unwritable today (`CanonRecorded` carries no `detail`) and waits on a `detail`-bearing event or DL-09's read seam (M14). Any player can become a facilitator by starting a scene (`commands.rs:102-104,160`); a seal by anyone else attests nothing.
- **D-E6-8 The named adjudicator is the run's settlement authority** (answer 6), replacing each condition's own attestor set for canon and people alike (08c §5.1 dev. 4, §6.1 dev. 3).
- **D-E6-9 A fence escrows before it takes custody** (answer 15): `+fence/deliver` is refused while the contract is `Offered`, because the `DepositInto` lock exists only for a live contract.
- **D-E6-10 A fence buys custody, never title** (answer 13): the deliverable is `DepositInto`; the title record and any `Disputed` encumbrance survive settlement; laundering is later and explicit.
- **D-E6-11 A fence quote is computed only from observable inputs** (answer 13) — hidden provenance contributes nothing, not even a discount, so a quote is never an oracle.
- **D-E6-12 Representation is catalog policy** (answer 9, doc 10 T5): `Instrument | Lot | Asset | Entitlement | Installed`, by category and significance with price as a hint. Lot lineage is append-only; coarsening rules past the starter pack are E7's.
- **D-E6-13 Every quote declares the clock its expiry is on** (answer 11); a stale definition version or observed revision refuses deterministically rather than re-pricing.
- **D-E6-14 The public projection is aggregate and shares the existence-hiding renderer** (answers 11, 20; doc 10 §14): no ids, no counterparties, bucketed depth.
- **D-E6-15 NPC market makers and campaign budgets are explicit and exhaustible** (doc 10 7A): a maker that runs out stops quoting, and nothing recapitalizes it without an attributed act.
- **D-E6-16 Payment never edits a sheet** (answer 12): settlement writes an `InstalledAugmentation`; the ruleset's own installation action — later — writes the sheet.
- **D-E6-17 The run board is ruleset-side, not `mush-market`** (answers 20, 24): a run offer is a contract with an audience notice, not a listing of goods, and E6a ships two cuts before `mush-market` exists. Moving it later is a projection change, not a data migration.
- **D-E6-18 `report_stolen` is a `mush-economy` action** (answer 14, 08e §4 item 6) — generic, Shadowrun-motivated, relaxing E3 invariant 19 for the title owner, an authorized insurer, or an authority; `Encumber` otherwise stays staff-only.
- **D-E6-19 The run's issuer is its payer** (M1). `@contract/amend` is issuer-only and A′ keeps only the amending party's acceptance, so a fixer-issuer would refund the payer's escrow at every `+run/crew accept` and leave a `Listed` entry standing on nothing. Personal: poster = payer, from their own account. Org: `scope=` makes the org the issuer; the posting officer holds `contract.draft`, `contract.accept` and `economy.account.post` on the scope. A fixer is `roles.author`/`representative` with a fee row, never the issuer. The roles rule in §3.3 (owner, 0.3) governs which roles may coincide.
- **D-E6-20 E6a settles by attestation only** (M2): outcome conditions are `Attestation`, `evidence=attest` is forced, `adjudicator=` defaults to `party:<payer>` (the shipped rig's shape, `e4b2.rs:209`), and `+run/post` refuses `before`/`after`/`fact` rows — `Fact` resolves `None` with no `sr.*` provider, no canon kind is registered until E6b, and a bare `Before` row is `Ready` the moment the last runner accepts and the sweep would pay it. The other routes arrive with E6b (§4.2).
- **D-E6-21 `ApprovalRule` gains a journal-derived rolling `window { total, period_secs }`** (DL-10, 08g §12.1 item 2): the account's debits plus placed holds over the trailing period, a refund never re-opening it; over the window ⇒ `NeedsApproval` on the threshold's own path. It closes the ≤-threshold drip against an org treasury (M3). No default — an account without a `window` keeps today's behaviour; organization run offers stay on. **Amended by the E6a step 1 verify fold (M1):** the window is a *discretionary-spend* control, and a **standing obligation's period post is exempt from it** — the debit was consented to when the obligation was created and the sweep can collect no approval, so a windowed treasury with a full window used to have its rent refused, the record suspended and a level-2 page opened with the creditor unpaid. The exemption hangs off the **authority** (`AuthEval::window_exempt`, `true` only for the sweep's `ObligationAuth`), never off the plan's caller-settable `cause`; the **threshold** clause is untouched, and the obligation's own debit still counts toward the account's window for every later discretionary spend.
- **D-E6-22 A crewed run's deadline disputes, never refunds** (M7): posted with `on_expiry = Dispute`, so a Johnson who simply never attests cannot recover the escrow by silence — bound parties are paged and agree; an unbound (`Listed`/`Crewing`) run refunds regardless. The board's `expires_at` is the contract's `consent.deadline` on `RealUtc` (M8): nothing else expires an `Offered` contract holding escrow, and `page_stale_contracts` ignores `Offered`.
- **D-E6-23 The lifestyle schedule's `maximum_catch_up` is 1, and a downgrade is a replacement** (M11, G12): one fire is one attempt, so a downtime backlog can never be the second miss; the `sr.month` clock's own `catch_up` (3) forgives on the clock first; the second miss replaces the record at the new tier through `create_obligation` with `unpaid_periods` carried, and `+lifestyle/set` is refused in arrears.

## 9. Constants

`MAX_PARTY_SIZE` 8 · `MAX_PARTY_INVITES` 16 · `MAX_PARTY_NAME_BYTES` 48 · `PARTY_IDLE_TTL` 7 d ·
`MAX_OPEN_RUNS_PER_PRINCIPAL` 4 (inside E4's `MAX_OPEN_CONTRACTS_PER_ISSUER` 16) · `BOARD_CAP` 128
open listings · `BOARD_PAGE` 20 · `BOARD_TTL` 14 d — **the contract's `consent.deadline` on
`RealUtc`** (M8), posted with `on_expiry = Dispute` (M7) · `MAX_RUN_OUTCOMES` 5 (inside E4's
`MAX_OUTCOMES` 8) · lifestyle tiers `street|squatter|low|middle|high|luxury`
(`crates/ruleset-shadowrun/src/data.rs:225-232`), one downgrade step per unpaid period, arrears
notices capped at 3 then silent until the level-2 escalation · **`LIFESTYLE_CATCH_UP` 1** — the
lifestyle schedule's `maximum_catch_up` (M11, D-E6-23) · `MAX_OUTCOME_NOTE_BYTES` 240 ·
`QUOTE_TTL_LIVE` 1 h (`RealUtc`) · `QUOTE_TTL_REMOTE` 24 h · `MAX_LISTINGS_PER_VENDOR` 64 ·
`MAX_LOT_PARENTS` 16 · `MAX_CATALOG_ENTRIES_PER_PACK` 512 · `DEPTH_BUCKETS` 8. No longer a
constant: `STALE_CONTRACT_AGE_SECS` (7 d) becomes `[economy].stale_contract_age_secs` and the paging
skips a contract with an unlapsed deadline (DL-08). `ApprovalRule.window` has **no default** — an
account without one keeps today's threshold-only behaviour (DL-10).

**`sr.month` and the two catch-ups (G12, D-LBK-7).** `sr.month` is created at runtime by `@sr/clock`
over `create_cadence` (`api.rs:1365-1425`) and stays a legacy `EconomicClock::Cadence(id)` until the
convergence registration makes it a declared cadence (`[economy.cadences]`, 08h §2 Q6). Two caps
apply, and both are said so neither is mistaken for the other: the clock-level `catch_up` (3, §11 q1)
*forgives* on the clock (`clock.rs:276-295`); the lifestyle schedule's `maximum_catch_up` (1) decides
*attempts* in one fire.

**Not constants — `[shadowrun.economy]` config** (answer 16): `enabled`, `instrument`,
`lifestyle_clock`, `lifestyle`, `player_run_offers`, `organization_run_offers`, `chargen_shopping`,
`catalog_paths`, plus `lifestyle_costs` per tier, `quote_ttl_live`/`quote_ttl_remote`,
**`fence_rate_immediate`/`fence_rate_consign`/`fence_delay_secs`**, `active_offer_cap`, and
`chargen_carryover_cap` (parsed but **unapplied** until shopping ships — answer 17). Parsed host-side
like every other block: `mush-config` `ShadowrunCfg` (the `EconomyCfg` pattern,
`crates/mush-config/src/config.rs:334-358`) → `mush_server::config_ext::shadowrun_config`
(`crates/mush-server/src/config_ext.rs:41` is the precedent) → `ShadowrunRuleset::with_config`
(`crates/ruleset-shadowrun/src/lib.rs:126-135`, which has **no external caller today**;
`modules.rs:315` constructs with `::new()`).

**Partial-switch table** (G9; E5c fold D2's `careers=false` precedent). Every row says what it stops
and what it never touches; no row ever touches an account, a balance or a live contract:

| Switch | `false` stops | Never touches |
|---|---|---|
| `enabled` | registers the verbs, creates nothing, evaluates nothing (E5c dev. 1) | existing accounts, balances, live contracts and obligations — they settle through E4 as before |
| `player_run_offers` | `+run/post` from a personal account, refused in the off-mode words | listings by orgs or staff; live runs |
| `organization_run_offers` | `scope=` posts | live org runs — they settle |
| `lifestyle` | no lifestyle obligation is created; existing ones are cancelled by `@sr/migrate` (never by boot); tiers stay cosmetic | accounts, balances, arrears already posted |
| `chargen_shopping` | parsed, inert until E6c | — |
| `fence_rate_*` | parsed, inert until E6b | — |

## 10. The owner's answers (08e §3, 2026-09-03 — binding)

1. `sr.nuyen` at scale 0; after chargen the account is authoritative, the sheet renders a derived balance; `system:sr.chargen` funds starting characters.
2. Karma stays separate — progression currency on its existing approval flow, never economic money.
3. Lifestyle is an automatic E4 obligation on an explicit `sr.month` campaign clock; autopay default; one failed period → notice/grace, a second may downgrade; downtime or a clock jump must never cause multiple surprise downgrades; physical housing stays separate.
4. Players, authorized org members, staff, player STs and campaign systems may create runs; **nobody from an uncontrolled faucet**; a Johnson is the visible representative, not necessarily payer or author, and the contract records the accountable principal.
5. The team is an ephemeral party, not an organization or principal; acceptance freezes a roster snapshot and payout shares; substitution needs an amendment and affected-party consent; payouts settle to individual accounts.
6. A typed scene-outcome / attestation seam — sealed scene outcome, authorized attestation, deterministic condition, or composite; **the offer names its adjudicator and permitted evidence before acceptance**; prose alone never releases escrow.
7. Freeze the split when the roster locks; exact shares must sum; **no unspecified remainder to the fixer**; separate frozen slots for payout, fee, advance, reserve, bonuses/penalties; fixed advance first, receipts later.
8. A bounded, data-driven outcome table in frozen terms — `failure`/`partial`/`success` as defaults, never hard-coded; each row names payouts, expense treatment and asset consequences.
9. Representation is catalog policy: important/serialized/restricted/upgradeable/stealable goods become assets; ammunition, drugs, reagents and commodities become bounded stacks; licences and programs may be entitlements; cyberware becomes an installed record retaining provenance.
10. A small, representative, source-controlled starter catalog; full catalogs are versioned packs; entries carry stable ids, definition versions, baseline price, availability, legality, representation kind, stackability, tags and rules references; vendors never mutate entries.
11. Vendor accounts, quotes and contracts; stock, a registered market source, or delayed procurement; **quote expiry declares its clock** (~1 h live, ~24 h remote); scarce or restricted goods use an engine-resolved recorded test; ST attestation only for authored availability.
12. Cyberware and clinics are a later slice; **payment alone never edits the sheet**; completion invokes a ruleset-owned installation revalidating Essence, prerequisites, custody, clinic capability, payment and sheet revision atomically; Karma costs are never bypassed.
13. Deterministic first-pass fence pricing with strict visibility; **title ≠ possession** — a fence may pay for custody while title and the theft claim stay attached; laundering is later and explicit.
14. A typed `report_stolen` by the title owner, an authorized insurer, or an authority, with public / restricted / covert visibility; later transfers generate evidence, not guilt; heat and notoriety later.
15. Configurable delayed fence settlement (24–72 h, shown in the offer); **the fence reserves or escrows before taking custody**; immediate-for-less vs consignment-for-more; notices only on custody, settlement, cancellation or dispute.
16. `[shadowrun.economy]` owned by the Shadowrun configuration (`enabled`, `instrument`, `lifestyle_clock`, `player_run_offers`, `organization_run_offers`, `chargen_shopping`, `catalog_paths`, plus quote expiry, offer caps, fence delay and lifestyle costs).
17. Chargen shopping and the 5,000¥ carry-over cap with **one live cash authority**: approval atomically opens the account, mints the priority allotment, pays chargen vendors, creates inventory/titles/installed records and leaves no more than the cap. **Until shopping exists, keep current behaviour and do not apply the cap.**
18. Migration is idempotent and inspectable, funded by `system:sr.migration`: a dry run shows each planned mint; the commit binds the old sheet value and revision, mints once, and stops treating the sheet number as authoritative; re-running returns the original result.
19. Cuts by actual prerequisite, and **no team vaults**: E6a after E4b2 (accounts, migration, lifestyle, representation, quotes, direct vendors, chargen shopping); E6b after the attestation seam (board, authored runs, parties, frozen rosters and splits, outcome settlement, basic clinics); E6c after knowledge support (belief-sensitive fences, heat, notoriety, investigations, title laundering).
20. A neutral `mush-market` crate owning bounded catalogs, listings, quotes, orders, expiry and vendor interfaces; the economy keeps accounts, contracts, title and vaults; rulesets own prices, availability, legality, item meaning and content.
21. A lightweight party subsystem — leader, accepted members, pending invitations, optional roles, revision, lifecycle, expiry. Not a principal; owns no money or title.
22. Separate run author, payer, Johnson, facilitator and adjudicator; they may coincide, but the data model must not assume it.
23. LLMs draft; authorities commit. An LLM ST may generate seeds, dialogue, complications and proposed outcomes; a registered campaign policy may authorize bounded recurring offers from a capped system budget; the LLM cannot invent funds, alter frozen terms, or settle on narration.
24. Bound the player-created run board: escrow required at publication, active-offer caps per principal, expiry of abandoned listings, and public / faction-visible / invitation-only runs.

## 11. Open questions (defaults chosen — **accepted by the owner 2026-09-03**; q2 resolved to E5b; all binding as written)

Only what the twenty-four answers and doc 10 leave open. Each default is what the build takes unless
the owner says otherwise.

1. **`sr.month`'s source and period.** Default `CadenceSource::WorldIc`, a 30-IC-day period, `catch_up_cap` 3. Answer 3 requires an explicit campaign clock and forbids a literal calendar; it does not pick the source. A `Manual` cadence advanced by staff is the alternative. *(0.3, G12: `sr.month` stays a legacy `EconomicClock::Cadence(id)` until D-LBK-7's convergence; this `catch_up` forgives on the clock, the lifestyle schedule's `maximum_catch_up` 1 decides attempts — §9.)*
2. **Who owns `@econ/cadence`.** Resolved: **E5b ships it** in `mush-economy` (its build brief already does — housing assesses on a cadence); E6a tests E4b1's B6/S2 against real clocks.
3. **08e answer 19's E6c list vs doc 10 §15.** Answer 19 puts belief-sensitive fences, heat, notoriety, investigations and title laundering in E6c "after knowledge support"; doc 10 §14–§15 replaces that gate with the **safe market projection**, which supplies no beliefs. Default: **E6c is the market**, and the five belief-dependent items move past E6 (§1.1). A real divergence between two binding documents, flagged rather than silently resolved.
4. **Where the run board lives.** Default `ruleset-shadowrun` (D-E6-17); `mush-market` listings are goods, a run is a contract with an audience notice.
5. **Whether a party may hold more than one live run.** Default **no** — `Locked { contract }` is singular, which keeps the frozen roster unambiguous.
6. **Fence rate shape.** Default: two configured permille rates against the catalog baseline, modified by declared legality/demand factors — config, not constants (answer 16, §9).
7. **Whether `+run/post` may name an org account the poster cannot view.** Default **no**, matching E4b2's `@contract/pay` rule (a listed party adds their own consideration — A1's fix, 08c §5.1).
8. **Starter catalog size.** Default ≤ 64 entries in one `sr.core` pack — enough to prove all five representations and both availability paths (answer 10's "representative, never exhaustive").
9. **Whether chargen shopping ships in E6a or E6c.** Default **E6c**, with E6a shipping only the cash authority (§3.5). Answer 19 lists shopping under E6a, but there is no catalog to shop from until `mush-market` exists, and answer 17 explicitly sanctions "keep current behaviour" meanwhile.
10. **Insurer identity for `@asset/stolen`.** Default: an org holding a new `asset.insure` capability on the asset's scope; "an appropriate authority" is staff in E6.

## 12. Review outcome

**Draft 0.1 → 0.2 (2026-09-03, one pass).** Two amendments: (1) the `scene.outcome` seal is no longer a
per-run staff gate — the contract's named adjudicator signs, staff conferral is the fallback (§4.1,
D-E6-7); (2) `@econ/cadence` is E5b's (§11 q2). Confirmed as correct against the tree: D-E6-4 (a
`System` principal can never be a contract party), the entities-named-at-seal rule, the doc 10 §15
re-gating of E6c on the safe market projection, and escrow-at-accept via the payer's own accept.
Open for the owner: §11 q1, q3–q10.

**Draft 0.2 → 0.3 (2026-09-04): the three-pass review** — adversarial money-safety, code-grounding at
HEAD `ce43714`, and design-fit against 08 / 08a / 08c / 08d / 08e / 10 (08g §11 item 8, DL-02).
M1–M19 and G1–G12 folded; the roles rule amended (owner, §3.3). What the review overturned: §2's
"whole list" (E6a needs five economy seams that do not exist — A′, the rolling window,
`create_obligation`, the contract seams, `InstrumentDisplay`); D-E6-4's "never a party" (a `System`
payer is unfundable, not unofferable — M4); D-E6-7's economy-side `sealed_by` check and `scene:{uid}`
source (M14); D-E6-3's downgrade path (replacement; catch-up 1 — M11); faction boards as an
`Audience` (M5); a deterministic E6a settlement route (M2); the issuer ≠ payer assumption (M1). Owner
additions: the roles rule, DL-04/DL-05/DL-08 and the M4 fold as E6a items, D-E6-19..23. Eight drifted
`file:line` cites corrected. The E6a brief (`e6a-brief.md`) carries the same corrections.

**Draft 0.3 → 0.3a — K answers folded (2026-09-04).** The K side's answers to 08i §7, with the
owner's rulings (08i §9), fold into three places: §2 confirms items 3–4's owner-exported seams as
09 §C10 delegation and commits every E6 mutating verb to the pinned rule-1 allowlist as
`legacy-post-image` in the cut that adds it (answers 16, 12); §4.5 gains E6b's scene-clock
monotonicity test — generation 1 for the record's life, an `ic_floor` raise never bumps it,
`SceneClockProvider` read-only (answer 9; E6b owns the `ic_floor`); §6's "`cargo fmt` stays off"
becomes fmt-clean per cut (`rustfmt.toml`, CI Format blocking since `91208ab`). Nothing else in 0.3
moves.
