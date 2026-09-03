# Economy / org / ownership — build status & E4 handoff

**Companion to [08-economy-organizations-ownership.md](08-economy-organizations-ownership.md)** (the
design; prefer it for *why*) and [08b-e3-implementation-plan.md](08b-e3-implementation-plan.md) (the
E3 plan as reviewed and as built — prefer it for *how* E3 fits together). This doc is the **build
ledger**: what of the E0–E8 plan is actually shipped, the decisions made while building it, the
live seams a later milestone consumes, and the concrete pickup point for **E4**. Read this first
when resuming economy work.

E0–E3 are on `origin/main` (the E3 commits, `99f5b9c` through the post-E3 module-order commit, were
pushed 2026-09-03). Each cut ran the standing rhythm — design-review-before-build, adversarial-verify — with
one variation for E3: by owner instruction the adversarial verify ran **once, over the whole E3
diff, after E3d**, and its confirmed findings were folded in a separate commit (see the last
Shipped row) rather than per cut.

## Shipped

| Cut | Crate / area | Commit | One line |
|-----|--------------|--------|----------|
| **E0** | `mush-core::ledger` + server durability | `565ff1b`, `32ca322` | ack-gated Ledger seal/rollover: byte-scan retained-write table re-emits un-acked sealed chunks, poison-seal fail-closed, boot contiguity |
| **E1a** | `mush-org` (pure library) | `6e9a030` | id vocab + records + `OrgState` + the pure `authorize()` engine + closed `OrgMutationPlan` |
| **E1b** | `mush-org` `OrgModule` | `11090aa` | persistence, fail-closed boot, `authorize_in_world`/`submit` seam, audit ledger, destroy scrub |
| **E1c** | `mush-faction` `FactionStandingProvider` | `97afebf` | faction member/officer/intel resolved into org policy predicates |
| **E2a** | `mush-economy` (pure library) | `8bed472` | checked `Amount`, the journal model, `validate`→`apply`, SHA-256 `plan_hash`, idempotent reversal |
| **E2b** | `mush-economy` `EconomyModule` | `547aec6` | journal on the E0 Ledger, atomic post bundle, **strict durable money gate**, directional boot self-heal |
| **E3 plan** | docs | `99f5b9c` | 08b — three-pass-reviewed plan (adversarial, grounding, design-fit) |
| **E3a** | `mush-economy` (pure) | `e780fd7` | `EconomicAsset`/`Vault`/`ApprovalRule`/`ApprovalRequest`, `AssetOp`/`CustodyOp` in the plan, invariants 10–22, `plan_hash` v2 (E2-identical for money-only plans), `NeedsApproval`, the `CustodyEval` + extended `AuthEval` seams |
| **E3b1** | `mush-economy` module | `288f769` | asset docs as journal projections (+ the `frozen` latch), vault docs, the `CustodyProvider` seam + `ContainerProvider`, `@asset`/`@vault`, destroy gate + `ObjDestroyed` backstop + boot liveness/custody reconciliation |
| **E3b2 (org)** | `mush-org` | `cb7c8f3` | `OrgStatus`/`ScopeStatus` enforced in `authorize`, `RetireScope`, scope cap, capability vocabulary, `ScopeUseProvider`, `@org/scope`, `@org/grant scope=/limit=` |
| **E3b2** | `mush-economy` module | `ae3de96` | org-scoped wallets, approvals re-qualified live at post, system-debit gate in the bridge, non-staff reads under existence hiding, `@account/close|unfreeze|approval`, `@econ/approve|deny|approvals` |
| **E3c (sim)** | `mush-sim` | `d48b1eb` | `InteriorCustodyProvider` — vaults over interiors; economy boots after sim |
| **E3c (tactics)** | `mush-tactics` | `fe4faea` | `SetComponentAsset` + the M33-deferred `asset` scrub on destroy and at boot |
| **E3d** | `mush-faction` | `d8d17c3` | `Faction.org` + `migrated`, `@faction/org|migrate`, view-gated balance read-through, destroy refused while bound |
| **E3 verify fold** | economy + faction | `fa97479` | the end-of-build adversarial verify's confirmed findings, each with a regression test (§"E3 verify outcome") |

Test tally after E3 (post-fold): **mush-org 46**, **mush-economy 157**, **mush-sim 72**,
**mush-tactics 52**, **mush-faction 56**, **mush-server 62**; `cargo test --workspace --no-fail-fast` runs 2012 passing. The one
failing workspace test, `mushd::web_e2e::spa_and_healthz_are_served`, predates E3 (it needs a
`web/dist` build). `cargo fmt` is **not** applied anywhere in E3: the tree had 1334 rustfmt hunks at
`d7f6707`, so CI's fmt step was already red before E3.

## The live seams a later milestone consumes

- **Authorization (read):** `mush_org::authorize_in_world(&World, &AuthRequest) -> Decision`. Pure
  engine underneath is `mush_org::authorize(&OrgState, &AuthRequest, &dyn PredicateEval)`. Since
  E3b2 it denies `Inactive` for a Retired scope or a non-Active org before any grant/policy work.
- **Org mutation (write):** `mush_org::submit(&mut Ctx, &OrgMutationPlan, OrgAction)`; ops now
  include `RetireScope`. Id minting is `OrgStore::alloc_*`.
- **Org wiring hooks (frozen at server assembly):** `OrgModule::with_predicates(..)` (E1c),
  `.with_capabilities(BTreeSet<CapabilityId>)` (E3b2 — the vocabulary `@org/grant` checks;
  `mush_economy::CAPABILITIES` is the economy's list), `.with_scope_use(..)` (E3b2 —
  `ScopeUseProvider`, implemented by `mush_economy::EconomyScopeUse`, so a bound scope cannot be
  retired).
- **Money + title + custody (write):** `mush_economy::post(&mut Ctx, &EconomyPlan)` — the plan
  carries `requested_lines`, `asset_ops`, `custody_ops`, `expected_revisions`,
  `expected_asset_revisions`; one durable bundle. `reverse(..)` reverses money lines only and
  refuses an effect-carrying original. **`post` is a handler's last fallible call** (a later `Err`
  discards the persists but not the world/store mutation).
- **Pure engine:** `validate(&plan, &snapshot, &ValidateEnv { auth, custody, now })`; the seams are
  `AuthEval` (money + `authorize_asset` / `authorize_title_receipt` / `authorize_vault` /
  `authorize_maintenance` / `authorize_approver`, all default-`false`) and `CustodyEval`.
- **Custody providers:** implement `mush_economy::CustodyProvider` and wire via
  `EconomyModule::with_custody(vec![..])`; the built-in `ContainerProvider` is always last;
  `mush_sim::InteriorCustodyProvider` is the precedent. Exactly one provider resolves an op (the one
  that `handles` the backing), refused if any other `claims` the object; moves ride the
  `CarryAttempt` hook chain. `post` admits **one custody effect per plan** at the module layer.
- **Destruction:** `mush_core::behavior::DestroyGate` registered with `reg.destroy_gate(..)`
  (economy's `EconomicDestroyGate` is the second user after sim's); the `ObjDestroyed` observer is
  the backstop and boot liveness the third leg — only `@destroy` emits the event.
- **Approvals:** `Account.approval: Option<ApprovalRule>`; `post` → `PostOutcome::NeedsApproval(id)`;
  `@econ/approve|deny`; approvers are re-qualified live at consumption; one live request per
  `plan_hash`; the consumed id rides the journal entry (`consumed_approval`).
- **Reads:** `mush_economy::may_view_account(w, actor, AccountId)` is the single view gate (staff,
  owner, `economy.account.view` on the scope); every refusal renders through the existence-hiding
  helpers in `commands.rs`.
- **Faction linkage:** `mush_faction::backing_org(w, &Faction) -> Result<OrgId, LinkError>` is the
  one resolver (`Unbound` / `Missing` / `Conflict`).
- **Durable gates (core):** `gate_on_durable_ack_strict()` for money; the non-strict
  `gate_on_durable_ack()` for the E3 control-doc writes (approval rules, vault/account/asset status).

## Decisions made while building (not all in the original doc 08 draft)

1. **Org creation is staff-only** (E1). Player-created orgs wait for a milestone with an economy
   consumer (neutralizes global-id / name-collision existence oracles). Delegated per-org admin
   (`org.role.manage`) still ships. *(E3: a shared player haven or team stash is therefore a
   staff-minted org; a non-org vault is owner-only + staff.)*
2. **`PrincipalId::Object` is clean** — no embedded generation. ObjIds are never reused, so the
   "reused id" threat is false; a destroyed object is scrubbed, not shadowed by a generation guard.
3. **`ScopeSelector` is a closed enum** — `Exact` / `SubtreeFrozen{snapshot_revision}` / `SubtreeLive`.
   Only `SubtreeLive` widens to later children (explicit); depth is capped at creation (≤8), so a
   containment walk terminates and an over-deep check fails closed. *(E3b2: scopes per org are
   capped at 64; `@org/scope create` stamps `created_rev = revision + 1` because
   `Organization.revision` is otherwise inert — a proper revision-bump op is a follow-up.)*
4. **Delegation fully re-resolves its live source each call** (never a captured `source_revision`),
   and the grantor must actually hold the source grant (both enforced at construction and re-checked
   live). Delegation is narrow-only.
5. **Faction is NOT fully migrated onto org (E1c scope cut).** E1c shipped only the *predicate
   provider*; the faction level/officer/intel authorization model stays faction-side (R-D1).
   Membership-mirroring into `OrgMembership` was **dropped**. *(E3d closed the deferral: the
   Faction→backing-`OrgId` mapping is `Faction.org`, and the "non-destructive membership adapter"
   is **struck** — see 20.)*
6. ~~The org capability vocabulary is a wiring hook, not yet enforced.~~ **Enforced since E3b2**:
   `OrgModule::with_capabilities` freezes the vocabulary at assembly (empty = accept all);
   `@org/grant` refuses an unknown capability and `limit=` on a non-amount capability.
7. **The journal is the sole economic authority.** Balances, the idempotency index, the reversed set,
   the txid index, and the allocators are **re-derived from the journal at boot** (R-B2) — none is a
   separately-persisted boot authority, so a companion-doc drop can never double-post. *(E3: asset
   title/assignment/encumbrance/lifecycle are journal-authority too — every change is an
   `AssetEffect` carrying the full post-state inside a journal transaction, and `asset:{obj}` docs are
   projections with the Account heal/freeze rules.)*
8. **Boot reconciliation is directional** (R-B1): a projection merely *lagging* the journal silently
   re-derives to the head; only real *drift*, an *ahead* projection, an *unreplayable* journal, or a
   per-instrument *conservation break* freezes / fails. Whole-economy freeze is the E2 granularity.
   *(E3: asset drift/ahead also latches the whole-economy freeze; a missing asset doc rebuilds
   **frozen**; an unjournaled asset doc is not loaded. Effect-only entries contribute nothing to
   conservation. **The whole-economy latch is now durable** — it rides `meta` and survives the boot
   that heals the drifted docs; `@econ/unfreeze` clears it. Before the E3 fold it was lost at the
   next boot, an E2-era gap.)*
9. **`Amount` forbids operator arithmetic** — no `Add`/`Sub`/`Neg`, only `checked_*`; serde-transparent
   `i64`; the client wire form is a decimal string (never the raw i64). `MAX_SCALE` caps render.
10. **Reversal is engine-only.** A plan carries no caller-settable `reverses` field; a reversal is a
    dedicated path fed a real transaction. Deterministic `econ.reverse/{id}` key + `is_reversed` guard
    make it idempotent + at-most-once. *(E3: a reversal reverses money lines only, never evaluates an
    `ApprovalRule` (an approval rule must not be a clawback shield), and **refuses** an original that
    carried asset/custody effects — title is restored by an attributed `@asset/transfer`.)*
11. **Strict durable gate for money** (invariant 9 / R-R1): the *posted* confirmation, AND an
    idempotency-HIT (`AlreadyPosted`) confirmation (R-R2), are emitted only after the durable ack; a
    dropped write suppresses them. *(E3: a `NeedsApproval` outcome and every control-doc write ride
    the non-strict gate.)*
12. **Durability checkpoint deferred** (R-D1): full-journal-replay-at-boot is the baseline; a
    persisted checkpoint lands when journal size makes boot cost matter. Cold-chunk *play-time*
    paging is deferred.
13. ~~No non-staff economic read in E2.~~ **A bounded non-staff read since E3b2** (D-E3-6): owners and
    `*.view` holders may `show`/`list` accounts, assets, vaults with a fixed field set; **every**
    refusal renders through an existence-hiding helper that collapses against *every* resource the
    error names ("No such account." for nonexistent, unviewable, and unviewable-but-refused alike);
    unauthorized list filters return empty; lists cap at 20. The journal stays staff-only; the
    disclosure *filter* is still E4.
14. **Holds / contracts / `SpendingProfile` are not built.** `Account.held_amount` is a projection
    fixed at zero (holds are E4); abstract WoD Resources are the E5 slice. Assets and vaults ARE
    built (E3).
15. **The `frozen` latch is doc-authority, outside every journal effect** (D-E3-1). A backstop
    freeze, `@asset/unfreeze`, vault/account status, and an approval rule are doc writes that touch
    **no journal-authority field, `revision` included** — bumping `revision` there reads as drift at
    the next boot (caught in test). Corrupt asset/vault docs fail the boot; a corrupt approval doc
    is skipped loudly but its id still floors the allocator.
16. **Approvals bind to `plan_hash`, live on the Account, and are re-qualified live** (D-E3-2).
    `ApprovalRule { threshold, policy }` per account; one live `ApprovalRequest` per hash (a second
    identical submission serves the incumbent); recorded approvers must still qualify at consumption
    (`economy.account.approve` on the scope + the role for `Threshold.from_role`; set membership,
    necessary and sufficient, for `NamedSet`; the approver must be live); the requester never
    counts, so `Threshold{1}` means one other person; `OneAuthorizedActor` is refused at set time
    and fails closed if stored; requests expire (24 h), are capped per requester (4) and per account
    (16), and GC at 7×TTL. An approval **reserves nothing** (holds are E4) and authorizes an
    intent, not a state (expected revisions are outside the hash). There is no asset-value
    escalation — two-party consent (E4) is that control.
17. **Title needs authority on both sides** (D-E3-12): source (owner / `asset.title.transfer` on the
    asset's scope / staff) **and** destination (an org → the capability on `to_scope`; another
    character → staff only). `Title` (create) and `Encumber` are staff-only in E3; `Release(Lien)` is
    open to its creditor. `TransferTitle` clears assignment and maintenance. `SetMaintenance` needs
    debit authority on the named account, which must be the owner's or in the asset's scope.
    Scope/owner coherence (invariant 18) ties every scoped resource to the org that owns it.
    **Authorization is asked about each account's gross debit, never its net** (a net-zero pair on
    one account was a free journal write — the fold's H1).
18. **Custody is a provider seam; one provider resolves a whole op** (D-E3-3). `CustodyProvider`
    frozen at assembly; the container provider runs `can_move_to` + the depth cap + the
    `CarryAttempt` hook chain + one `move_to`; the interior provider requires `sim.participant`,
    pre-validates the destination, and withdraws as `Unplace{keep:true}` + one `move_to` (the
    two-move form could strand the object with a false recovery-log line), running every carry hook
    except sim's own. Custody standing = carried (exact `Obj.location == actor`) or co-located as
    owner/assignee/controller/staff; **exits and actors are never custody** (an exit is filed in
    `exits`, invisible to every custody query); a vault's own backing cannot be deposited; a
    withdrawal lands only on the actor or the actor's location and never inside any vault's
    backing chain. Custody effects are audit-only; boot reconciles the last `Deposit` per object
    against the provider and freezes a titled mismatch per asset (a provider `None` is
    unverifiable, not a mismatch).
19. **Module order matters: economy boots after sim, and faction after org.** `SpatialState` is
    installed at registration, so an economy booting first sees an empty state and a definite "not
    held" for every interior deposit. Org still precedes economy; a server without
    `SimulationModule` takes the provider's `None` path safely. *(Owner decision 2026-09-03:
    `FactionModule` loads after `OrgModule` so faction boot can validate every `Faction.org` link
    against loaded org state — a missing org or a two-factions-one-org conflict is reported loudly
    at boot and never unbound; `backing_org` stays the at-use enforcement. The 24 h approval TTL
    and staff-only character-to-character title transfer were confirmed at the same time.)*
20. **The E1c membership adapter is struck** (D-E3-11). Role templates need real `OrgMembership`;
    a faction-backed org authorizes through direct grants and `FactionStandingProvider` predicates;
    `@account/approval` refuses `from_role` for a role the org does not have. A faction officer with
    no membership and no grant sees ratings and no money and cannot spend from the org wallet.
21. **Faction pools migrate by explicit, static, per-key marking** (D-E3-9). `Faction.migrated`
    decides what is money; registering an instrument never reclassifies a rating.
    `@faction/migrate <f>=<key> <sysacct> /confirm` takes the pool verbatim as minor units, posts
    first (key `faction.migrate/{fid}/{code}`), then marks; if the idempotency record already
    exists it marks and logs the residual instead of re-posting, so an edited pool still converges.
    `@faction/destroy` is refused while any link exists; two factions bound to one org both fail
    closed. Balances render in `+faction`/`@faction/show` only through `may_view_account`.
22. **`PolicyRef` is not built** (D-E3-5); per-resource policy is a leaf scope + `AccessPolicy`
    rules against `ScopeSelector::Exact`. `CatalogRef { ruleset, code }` is the opaque precursor of
    §9.4's `CatalogId` (E6 owns the reconciliation).
23. **The four-axis split holds** (D5): `@asset/assign|transfer` leave `StationAssignment` seats
    and `control_gen` untouched, and a station assignment leaves the title record untouched (tested
    in `mush-sim/tests/custody.rs`). A new title owner has **no** verb to evict an operator in E3.
24. **`@chown` and title are independent both ways** (§9.1); `@chown` is not gated. The destroy
    gate covers the target and everything it transitively contains (a live title, an active or
    non-empty vault backing, a principal holding a balance or a title); `@destroy/override` bypasses
    only `SAFE`, never the gate; the non-staff refusal is neutral text.

## E3 verify outcome (end-of-build, three lenses, probe-confirmed)

Confirmed and folded, each with a regression test: **(H)** net-zero self-transfer bypassing debit
authorization; **(H)** exits accepted as vault contents but invisible to every custody query;
**(H)** the whole-economy freeze latch lost at the next boot (E2-era); **(M)** cross-resource
existence leaks through the error renderer (pay destination, `@asset/maintenance` account);
**(M)** a withdrawal landing inside another vault's backing chain; **(M)** a vault's own backing
deposited into another vault; **(M)** no durable gate on the new control-doc writes; **(M)**
`@faction/migrate` unable to converge after a pool edit; **(L–M)** the migrate opening the org
account before its last fallible call; allocator floors persisting only one boot; `Account.revision`
moving backwards on a lag heal; actors depositable as vault contents; a destroyed `NamedSet`
approver still counting; approval caps checked after a stale-request retire; non-recovery control
verbs usable while frozen. Read-verified sound: `post` ordering and the terminal-fallible rule in
every command, the both-sides title rules, custody standing, approval consumption and hash
binding, reversal unforgeability, E2-era journals under the E3 binary (hash pin + live retry), latch
polarity across reboot, entry boundedness, key routing, module order.

## Pick up here: E4 — holds, escrow, obligations, and contracts

Design: doc 08 §8 (holds/clocks/obligations/contracts), §12 (visibility filter), §18 "E4".
Concretely, and what E3 left as the hooks:

- **Holds + `held_amount`.** `Account.held_amount` is a zero projection; `available()` exists.
  A hold is the E4 analogue of an approval that *reserves* (decision 16 says approvals reserve
  nothing). Two approved plans over one account both post today — holds close that.
- **Two-party consent.** E3 plans are single-authority (D-E3-4); a sale needs the buyer's debit
  and the seller's title in one plan — contracts/escrow carry the counterparty's authority. The
  record shape does not change: `asset_ops`/`custody_ops` already ride the journal.
- **`Encumbrance::Lease { lessor, contract }` / `Collateral { contract }`** — the enum is
  serde-tagged and open; `ContractId` is E4's. `Encumber` becomes non-staff when a lien has a
  contract behind it.
- **Clocks + bounded catch-up** (§8.2/D7) — no scheduler work exists in the economy yet.
- **The disclosure filter** (§12.2) — E3's existence-hiding renderer and `may_view_account` are the
  hooks; `EconomicVisibility` is recorded, not enforced; the journal is staff-only.
- **Approval → org audit.** Approve/deny decisions live on the request doc; nothing lands in org's
  audit ledger (§12.3 wants it).
- Follow-ups outside E4's core: an `Organization.revision` bump op + `SetOrgStatus`/`PutRole`
  (decision 3); per-asset isolation of the whole-economy freeze; the durability checkpoint (12);
  wiring `TacticsModule` into the server (the component-asset scrub is proved in-crate only);
  `CustodyRef::Registered` has no provider; the lingering non-quarantined `Unplaced` placement doc
  after an interior withdrawal (inert); an operator-eviction verb for a new title owner (E7).

## Build discipline (keep it)

Each cut: (1) a short design review / decision pass before building; (2) build the smallest
coherent library→module rhythm; (3) an adversarial-verify pass hunting fail-open / money-safety /
durability holes, **probe-confirmed** (the E3 verifiers wrote throwaway tests, which turned
plausible into confirmed and kept the fold honest); (4) fold every confirmed finding with a
regression test; (5) commit with the verify outcome in the message. E3's single end-of-build verify
found three high and several medium holes across four cuts — the per-cut cadence E0–E2 used would
have caught the E3b1 custody holes before E3c built on them; prefer per-cut verifies for E4.
