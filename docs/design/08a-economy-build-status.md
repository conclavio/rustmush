# Economy / org / ownership — build status & E3 handoff

**Companion to [08-economy-organizations-ownership.md](08-economy-organizations-ownership.md)** (the
design; prefer it for *why*). This doc is the **build ledger**: what of the E0–E8 plan is actually
shipped, the decisions made while building it, the live seams a later milestone consumes, and the
concrete pickup point for **E3**. Read this first when resuming economy work.

Everything below is on `main` and pushed to `origin`. Each cut ran the standing rhythm:
**design-review-before-build, adversarial-verify-after-build**; every verify-confirmed hole was
folded (with a regression test) before commit.

## Shipped

| Cut | Crate / area | Commit | One line |
|-----|--------------|--------|----------|
| **E0** | `mush-core::ledger` + server durability | `565ff1b`, `32ca322` | ack-gated Ledger seal/rollover: byte-scan retained-write table re-emits un-acked sealed chunks, poison-seal fail-closed, boot contiguity |
| **E1a** | `mush-org` (pure library) | `6e9a030` | id vocab + records + `OrgState` + the pure `authorize()` engine + closed `OrgMutationPlan` |
| **E1b** | `mush-org` `OrgModule` | `11090aa` | persistence, fail-closed boot, `authorize_in_world`/`submit` seam, audit ledger, destroy scrub |
| **E1c** | `mush-faction` `FactionStandingProvider` | `97afebf` | faction member/officer/intel resolved into org policy predicates |
| **E2a** | `mush-economy` (pure library) | `8bed472` | checked `Amount`, the journal model, `validate`→`apply`, SHA-256 `plan_hash`, idempotent reversal |
| **E2b** | `mush-economy` `EconomyModule` | `547aec6` | journal on the E0 Ledger, atomic post bundle, **strict durable money gate**, directional boot self-heal |

Test tally across the new crates: **mush-org 34**, **mush-faction +2** (standing), **mush-economy 27**.
The E2b core change kept `mush-core`/`mush-server`/`mush-mail`/`mush-jobs` green.

## The live seams a later milestone consumes

- **Authorization (read):** `mush_org::authorize_in_world(&World, &AuthRequest) -> Decision`. Pure
  engine underneath is `mush_org::authorize(&OrgState, &AuthRequest, &dyn PredicateEval)`.
- **Org mutation (write):** `mush_org::submit(&mut Ctx, &OrgMutationPlan, OrgAction)`. Id minting is
  the public `OrgStore::alloc_org/alloc_scope/alloc_role/alloc_grant/alloc_delegation/alloc_policy`.
- **Cross-module policy predicates:** implement `mush_org::OrgPredicateProvider` and wire it via
  `OrgModule::with_predicates(vec![...])` at server assembly (E1c's faction provider is the pattern).
- **Money (write):** `mush_economy::post(&mut Ctx, &EconomyPlan)` and
  `mush_economy::reverse(&mut Ctx, TransactionId, actor, cause)` — the ONLY balance-mutation paths.
  Account/instrument creation: `open_account` / `register_instrument`. Ids via
  `EconomyStore::alloc_account` (+ internal txid).
- **Economy authorization:** `mush_economy::post_capability()` = `economy.account.post`; the
  `WorldAuth` bridge already checks own-account / credit-is-free / scoped→org-policy / staff / system.
- **Durable money gate (core):** `Ctx::gate_on_durable_ack_strict()` — a money confirmation is
  suppressed (never delivered-with-warning) on a dropped durable write. `Deferred::DurableGate` now
  carries `strict`; the mail/XP gate stays non-strict (`gate_on_durable_ack`).

## Decisions made while building (not all in the original doc 08 draft)

1. **Org creation is staff-only** (E1). Player-created orgs wait for a milestone with an economy
   consumer (neutralizes global-id / name-collision existence oracles). Delegated per-org admin
   (`org.role.manage`) still ships.
2. **`PrincipalId::Object` is clean** — no embedded generation. ObjIds are never reused, so the
   "reused id" threat is false; a destroyed object is scrubbed, not shadowed by a generation guard.
3. **`ScopeSelector` is a closed enum** — `Exact` / `SubtreeFrozen{snapshot_revision}` / `SubtreeLive`.
   Only `SubtreeLive` widens to later children (explicit); depth is capped at creation (≤8), so a
   containment walk terminates and an over-deep check fails closed.
4. **Delegation fully re-resolves its live source each call** (never a captured `source_revision`),
   and the grantor must actually hold the source grant (both enforced at construction and re-checked
   live). Delegation is narrow-only.
5. **Faction is NOT fully migrated onto org (E1c scope cut).** E1c shipped only the *predicate
   provider*; the faction level/officer/intel authorization model stays faction-side (R-D1).
   Membership-mirroring into `OrgMembership` was **dropped** (the predicate seam covers authorization
   without a second membership store). **The Faction→backing-`OrgId` mapping + `Faction.resources`
   migration is deferred to E3**, where account ownership defines what that mapping must carry.
6. **The org capability *vocabulary* is a wiring hook, not yet enforced.** `OrgModule` accepts a
   frozen provider table; a *closed capability vocabulary* (reject grants of unknown caps) has **no
   consumer until a ruleset extends `economy.*`**, so its enforcement + cross-crate composition is
   deferred to when E-series rulesets need it.
7. **The journal is the sole economic authority.** Balances, the idempotency index, the reversed set,
   the txid index, and the allocators are **re-derived from the journal at boot** (R-B2) — none is a
   separately-persisted boot authority, so a companion-doc drop can never double-post.
8. **Boot reconciliation is directional** (R-B1): a projection merely *lagging* the journal silently
   re-derives to the head; only real *drift* (stored balance disagrees with the journal at its claimed
   seq), an *ahead* projection (lost tail), an *unreplayable* journal (contiguity breach / unknown
   account), or a per-instrument *conservation break* (R-R5) freezes / fails. Whole-economy freeze is
   the E2 granularity (per-account isolation is later).
9. **`Amount` forbids operator arithmetic** — no `Add`/`Sub`/`Neg`, only `checked_*`; serde-transparent
   `i64`; the client wire form is a decimal string (never the raw i64). `MAX_SCALE` caps render.
10. **Reversal is engine-only.** A plan carries no caller-settable `reverses` field; a reversal is a
    dedicated path fed a real transaction, so the back-reference can't be forged. Deterministic
    `econ.reverse/{id}` key + `is_reversed` guard make it idempotent + at-most-once.
11. **Strict durable gate for money** (invariant 9 / R-R1): the *posted* confirmation, AND an
    idempotency-HIT (`AlreadyPosted`) confirmation (R-R2), are emitted only after the durable ack; a
    dropped write suppresses them.
12. **Durability checkpoint deferred** (R-D1): full-journal-replay-at-boot is the E2 baseline (correct,
    precedented); a persisted checkpoint (covered_seq + state hash) lands when journal size makes boot
    cost matter. Cold-chunk *play-time* paging is deferred (no reader; boot reads all chunks from
    `data.kv`).
13. **No non-staff economic read in E2.** `EconomicVisibility` / `CausalRef` are recorded but NOT
    enforced; all E2 inspection is staff-gated. The disclosure filter is E4.
14. **Holds / assets / contracts / `SpendingProfile` are not built.** `Account.held_amount` is a
    projection fixed at zero (holds are E4); assets/vaults are E3; abstract WoD Resources are the E5
    slice.

## Pick up here: E3 — organization wallets, assets, and vaults

Design: doc 08 §5.4 (scopes/compartments), §9 (assets/title/custody/vaults), §18 "E3". Concretely:

- **Org wallets bound to scopes + approval enforcement.** `Account.scope: Option<OrgScopeId>` and the
  `WorldAuth` scoped-account path already call `authorize_in_world` with `economy.account.post` — E3
  makes the *divisional-wallet* vertical real (open org-owned accounts under a scope; enforce the
  capability + a per-transaction `ApprovalPolicy`, which is type-only in E1 today).
- **The Faction→backing-`OrgId` mapping + `Faction.resources` migration** (deferred from E1c). Decide
  the linkage shape *here*, driven by account ownership: a faction gains an `org: Option<OrgId>`;
  exact `Faction.resources` pools move into org economy accounts; non-conserved ratings
  (`influence.police`) stay faction-side.
- **Assets / title / custody / assignment / encumbrance** (§9.1–9.2): `EconomicAsset` records; the
  four-axis title/custody/**operation (= the shipped F3 control lease)**/maintenance separation (D5);
  the M32/M33 per-component `asset` pointer (the M33-deferred asset-scrub) resolves here.
- **Vaults** bound to ordinary containers and `mush-sim` interiors (§9.3).
- **Atomic payment + title + custody plans**, and programmatic-destruction recovery.

E3's authorization already has everything it needs: `authorize_in_world` (live), the `submit` +
`post` seams, and the `ApprovalPolicy` type. The main *new* engine work is the asset/title/vault
records + the multi-effect atomic plans that move money AND title AND custody in one durable bundle.

## Build discipline (keep it)

Ultracode is on. Each cut: (1) a short design review / decision pass before building; (2) build the
smallest coherent library→module rhythm (M31→M33 shape); (3) an adversarial-verify workflow hunting
fail-open / money-safety / durability holes; (4) fold every confirmed finding with a regression test;
(5) commit with the verify outcome in the message. The E0–E2 verifies each found 1–4 real holes that
this discipline caught pre-commit — do not skip it for E3 (assets + atomic multi-effect plans are
exactly where custody/title can desync from money).
