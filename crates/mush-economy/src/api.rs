//! The E2b public seam + persistence layer.
//!
//! [`post`] and [`reverse`] are the ONLY balance-mutation paths (the §18-E2 exit
//! criterion): each validates through the pure E2a engine, allocates ids, applies,
//! writes the whole bundle — the journal append + every touched account projection +
//! the allocator `meta` — on one `WriteBatch`, and gates the confirmation on the
//! **strict** durable ack (invariant 9 / R-R1). [`WorldAuth`] bridges the injected
//! [`AuthEval`] to `mush_org::authorize_in_world`, re-evaluated live at every post.
//!
//! Persistence (namespace [`NS`] = `economy`): `instrument:{id}`, `account:{id}`,
//! a `meta` allocator doc, and the journal under `log:journal:*`. The idempotency
//! index, the reversed set, and the txid index are NOT persisted separately — they
//! are re-derived from the journal at boot, so a companion-doc drop can never cause a
//! double-post (R-B2).

use mush_core::{
    Ctx, Deferred, MAX_CONTAINMENT_DEPTH, ModuleOp, ModuleWrite, ObjId, Outbox, Powers, Role,
    World, perm,
};
use mush_org::{
    ApprovalPolicy, AuthRequest, CapabilityId, MembershipStatus, OrgScopeId, OrgStatus, PrincipalId,
    ScopeStatus, authorize_in_world,
};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::custody::{CustodyError, WorldCustody};
use crate::engine::{APPROVAL_TTL_SECS, AuthEval, ValidateEnv, apply, plan_reversal, validate};
use crate::ids::{AccountId, ApprovalRequestId, InstrumentId, TransactionId, VaultId};
use crate::module::{EconomyStore, JOURNAL_LOG, NS};
use crate::plan::{ApprovalNeed, AssetOp, CustodyOp, EconomyError, EconomyPlan, PlanResult, Prepared};
use crate::record::{
    Account, ApprovalRequest, ApprovalStatus, CustodyDirection, EconomicAsset, EconomicCause,
    InstrumentDef, Vault,
};

/// The kv key of the allocator meta document.
pub const META_KEY: &str = "meta";

// ---- keys -----------------------------------------------------------------

/// `instrument:{id}`.
#[must_use]
pub fn instrument_key(id: &InstrumentId) -> SmolStr {
    SmolStr::new(format!("instrument:{}", id.as_str()))
}
/// `account:{id}`.
#[must_use]
pub fn account_key(id: AccountId) -> SmolStr {
    SmolStr::new(format!("account:{}", id.0))
}
/// `asset:{obj}` — the title projection (journal-authority fields; the `frozen`
/// latch is doc authority).
#[must_use]
pub fn asset_key(object: ObjId) -> SmolStr {
    SmolStr::new(format!("asset:{}", object.0))
}
/// `vault:{id}`.
#[must_use]
pub fn vault_key(id: VaultId) -> SmolStr {
    SmolStr::new(format!("vault:{}", id.0))
}
/// `approval:{id}` — a **leaf** doc (E3b2): a corrupt one is skipped loudly, never
/// a boot failure, because an unreadable approval can only ever cost someone a
/// re-submission.
#[must_use]
pub fn approval_key(id: ApprovalRequestId) -> SmolStr {
    SmolStr::new(format!("approval:{}", id.0))
}

// ---- persist helpers ------------------------------------------------------

pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(Deferred::Persist(ModuleOp { ns: SmolStr::new_static(NS), op }));
}
pub(crate) fn persist_instrument(out: &mut Outbox, def: &InstrumentDef) {
    push_op(out, ModuleWrite::JsonSet {
        key: instrument_key(&def.id),
        value: serde_json::to_value(def).expect("instrument serializes"),
    });
}
pub(crate) fn persist_account(out: &mut Outbox, a: &Account) {
    push_op(out, ModuleWrite::JsonSet {
        key: account_key(a.id),
        value: serde_json::to_value(a).expect("account serializes"),
    });
}
pub(crate) fn persist_asset(out: &mut Outbox, a: &EconomicAsset) {
    push_op(out, ModuleWrite::JsonSet {
        key: asset_key(a.object),
        value: serde_json::to_value(a).expect("asset serializes"),
    });
}
pub(crate) fn persist_vault(out: &mut Outbox, v: &Vault) {
    push_op(out, ModuleWrite::JsonSet {
        key: vault_key(v.id),
        value: serde_json::to_value(v).expect("vault serializes"),
    });
}
pub(crate) fn persist_approval(out: &mut Outbox, r: &ApprovalRequest) {
    push_op(out, ModuleWrite::JsonSet {
        key: approval_key(r.id),
        value: serde_json::to_value(r).expect("approval request serializes"),
    });
}
pub(crate) fn delete_key(out: &mut Outbox, key: SmolStr) {
    push_op(out, ModuleWrite::Del { key });
}
pub(crate) fn persist_meta(out: &mut Outbox, meta: serde_json::Value) {
    push_op(out, ModuleWrite::JsonSet { key: SmolStr::new_static(META_KEY), value: meta });
}
/// Wrap the journal `Ledger::append` write in the economy namespace.
pub(crate) fn persist_journal(out: &mut Outbox, op: ModuleWrite) {
    push_op(out, op);
}

// ---- the authorization bridge ---------------------------------------------

/// The economy capability an actor must hold to move value on a scoped account.
#[must_use]
pub fn post_capability() -> CapabilityId {
    CapabilityId::new("economy.account.post").expect("valid capability id")
}

/// Whether an approver still exists as a live world object (not missing, not
/// `GOING`) — the liveness half of [`AuthEval::authorize_approver`].
#[must_use]
fn approver_live(w: &World, who: ObjId) -> bool {
    w.get(who).is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
}

/// Staff: `SIM_ADMIN` or `Royalty` — the one staff predicate the whole crate uses.
#[must_use]
pub(crate) fn is_staff(w: &World, who: ObjId) -> bool {
    perm::has_power(w, who, Powers::SIM_ADMIN) || perm::has_role(w, who, Role::Royalty)
}

/// Whether `actor` currently holds `capability` on `scope` (`AuthRequest.amount:
/// None` — these are not amount-carrying operations, and a `GrantLimit` on a
/// non-amount capability never authorizes, by design).
#[must_use]
pub(crate) fn holds_capability(w: &World, actor: ObjId, capability: &str, scope: OrgScopeId) -> bool {
    let Some(capability) = CapabilityId::new(capability) else { return false };
    let req = AuthRequest { actor, capability, scope, now: w.now(), amount: None };
    authorize_in_world(w, &req).is_allow()
}

/// Invariant 18's **seam half**: a scoped resource's scope must exist, belong to
/// the organization named in its owner, and be `Active` — with that organization
/// `Active` too. The pure engine proves only the structural half (`scope: Some` ⇒
/// the owner is an organization); which organization, and whether it is live, is a
/// question only the org state can answer.
#[must_use]
pub(crate) fn scope_coherent(w: &World, scope: Option<OrgScopeId>, owner: &PrincipalId) -> bool {
    let Some(sid) = scope else { return true };
    let PrincipalId::Organization(org) = owner else { return false };
    let Some(store) = w.module_state::<mush_org::OrgStore>() else { return false };
    let state = store.state();
    let Some(sc) = state.scope(sid) else { return false };
    if sc.org != *org || sc.status != ScopeStatus::Active {
        return false;
    }
    state.org(*org).is_some_and(|o| o.status == OrgStatus::Active)
}

/// An object plus everything it transitively contains, depth-capped — what the
/// destroy gate must clear (§4.9: contents are dispersed, not destroyed, but a
/// container full of titled goods is still not a thing you may delete).
#[must_use]
pub(crate) fn subtree(w: &World, root: ObjId) -> Vec<ObjId> {
    let mut out = vec![root];
    let mut frontier = vec![root];
    for _ in 0..MAX_CONTAINMENT_DEPTH {
        let mut next = Vec::new();
        for id in frontier.drain(..) {
            for &c in w.contents_of(id) {
                if !out.contains(&c) {
                    out.push(c);
                    next.push(c);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out
}

/// Whether an org compartment may still **receive** value (§4.4). A `Dissolving`
/// org must be able to take final settlement — that is what dissolving *is* — while
/// a `Suspended` or `Closed` one must not, and neither may a compartment whose org
/// has vanished. An unscoped account takes credit freely, as in E2.
#[must_use]
fn credit_allowed(w: &World, account: &Account) -> bool {
    if account.scope.is_none() {
        return true;
    }
    let PrincipalId::Organization(org) = &account.owner else { return false };
    let Some(store) = w.module_state::<mush_org::OrgStore>() else { return false };
    store
        .state()
        .org(*org)
        .is_some_and(|o| matches!(o.status, OrgStatus::Active | OrgStatus::Dissolving))
}

/// Bridges the injected [`AuthEval`] to the live org authorization engine (R-D4).
/// Money-safe defaults: receiving value needs no authority beyond a live
/// compartment; you may move your own unscoped account; a scoped (org) account
/// defers to the live org policy; a system source/sink debit is **staff**.
pub(crate) struct WorldAuth<'a> {
    pub(crate) w: &'a World,
}

impl AuthEval for WorldAuth<'_> {
    fn authorize(&self, actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        let staff = matches!(actor, PrincipalId::Object(o) if is_staff(self.w, *o));
        // A credit (receiving value) needs no authority — but §4.4 gates WHICH
        // compartment may receive: dissolving settles, suspended/closed does not.
        if !delta.is_negative() {
            return credit_allowed(self.w, account);
        }
        // A6 — **the mint/burn gate, in the bridge**. A negative delta on a
        // registered system source/sink creates money out of nothing, so it is
        // staff, full stop. E2 left this to "the command layer", which fails OPEN
        // the moment `econ_pay`'s owner guard relaxes for divisional wallets (it
        // did, one function below): a non-staff payer naming a faucet as `from`
        // would have minted. The check belongs where every path passes.
        if matches!(account.owner, PrincipalId::System(_)) {
            return staff;
        }
        // Staff may post attributed moves on any other account (corrections, admin).
        if staff {
            return true;
        }
        // Org-scoped → the live org policy decides (E1 capability + amount limit),
        // and only over a LIVE compartment: §4.4 requires the org `Active` and the
        // scope `Active` for a debit. `scope_coherent` proves both, plus that the
        // scope really belongs to the owning org (invariant 18's seam half); org's
        // own `authorize` denies `Inactive` on the same facts after E3b2, so this is
        // defence in depth, not the only lock.
        if let Some(scope) = account.scope {
            let PrincipalId::Object(obj) = actor else { return false };
            if !scope_coherent(self.w, Some(scope), &account.owner) {
                return false;
            }
            let amount = delta.minor().checked_abs().unwrap_or(i64::MAX);
            let req = AuthRequest {
                actor: *obj,
                capability: post_capability(),
                scope,
                now: self.w.now(),
                amount: Some(amount),
            };
            return authorize_in_world(self.w, &req).is_allow();
        }
        // Unscoped → own it. (An unscoped org-owned account is unreachable: no
        // object principal can equal an `Organization`, so it fails closed.)
        *actor == account.owner
    }

    /// Invariants 19–20 (§4.4). Staff allow; otherwise the actor is the
    /// `title_owner`, or holds the op's capability on the asset's (live, coherent)
    /// org compartment. `Title` and `Encumber` are **staff-only in E3**;
    /// `Release(Lien)` is additionally open to its own creditor, `Release(Disputed)`
    /// is staff.
    fn authorize_asset(&self, actor: &PrincipalId, asset: Option<&EconomicAsset>, op: &AssetOp) -> bool {
        let PrincipalId::Object(a) = actor else { return false };
        let staff = is_staff(self.w, *a);
        match op {
            // Creating a title mints a compartment binding; never an incoherent one,
            // staff or not (invariant 18's seam half).
            AssetOp::Title { owner, scope, .. } => staff && scope_coherent(self.w, *scope, owner),
            AssetOp::Encumber { .. } => staff,
            AssetOp::Release { encumbrance, .. } => {
                staff
                    || matches!(encumbrance,
                        crate::record::Encumbrance::Lien { creditor, .. } if creditor == actor)
            }
            _ => {
                let Some(asset) = asset else { return false };
                if staff || asset.title_owner == *actor {
                    return true;
                }
                let capability = match op {
                    AssetOp::Assign { .. } | AssetOp::Unassign { .. } => "asset.assign",
                    _ => "asset.title.transfer",
                };
                self.scoped_capability(asset.scope, &asset.title_owner, capability, *a)
            }
        }
    }

    /// The destination half of D-E3-12: title carries liabilities, so receiving is
    /// not free the way a money credit is. To an org → `asset.title.transfer` on
    /// `to_scope`, which must be present and belong to that org. To an object
    /// principal → the actor IS that principal, or staff.
    fn authorize_title_receipt(&self, actor: &PrincipalId, to: &PrincipalId, to_scope: Option<OrgScopeId>) -> bool {
        let PrincipalId::Object(a) = actor else { return false };
        let staff = is_staff(self.w, *a);
        match to {
            PrincipalId::Organization(_) => {
                let Some(scope) = to_scope else { return false };
                if !scope_coherent(self.w, Some(scope), to) {
                    return false;
                }
                staff || holds_capability(self.w, *a, "asset.title.transfer", scope)
            }
            // A non-org principal carries no compartment (the pure engine refuses a
            // scope here too; this is the belt).
            PrincipalId::Object(_) => to_scope.is_none() && (actor == to || staff),
            PrincipalId::System(_) => to_scope.is_none() && staff,
        }
    }

    /// Vault policy (§4.4): the vault's owner, `vault.deposit`/`vault.withdraw` on
    /// its compartment, or staff. Vault ops never consult title (D5).
    fn authorize_vault(&self, actor: &PrincipalId, vault: &Vault, op: &CustodyOp) -> bool {
        let PrincipalId::Object(a) = actor else { return false };
        if is_staff(self.w, *a) || vault.owner == *actor {
            return true;
        }
        let capability = match op {
            CustodyOp::Deposit { .. } => "vault.deposit",
            CustodyOp::Withdraw { .. } => "vault.withdraw",
        };
        self.scoped_capability(vault.scope, &vault.owner, capability, *a)
    }

    /// Invariant 14: pointing maintenance at an account means "may DEBIT it".
    /// Crediting is free, so a zero-delta probe through [`AuthEval::authorize`]
    /// proves nothing — and a `System` faucet/sink is staff-only either way (A6).
    fn authorize_maintenance(&self, actor: &PrincipalId, account: &Account) -> bool {
        let PrincipalId::Object(a) = actor else { return false };
        if is_staff(self.w, *a) {
            return true;
        }
        if matches!(account.owner, PrincipalId::System(_)) {
            return false;
        }
        if *actor == account.owner && account.scope.is_none() {
            return true;
        }
        self.scoped_capability(account.scope, &account.owner, "economy.account.post", *a)
    }

    /// Invariant 17 / §4.4 — **re-run LIVE at every post**, never captured at
    /// approve time (A5): an approver who has since lost the capability, the role,
    /// or their place in the named set simply stops counting.
    ///
    /// The two policies ask different questions:
    ///
    /// - **`Threshold`** asks about standing: `economy.account.approve` on the
    ///   account's compartment (staff pass it by fiat), plus — for
    ///   `from_role: Some(r)` — a live `OrgMembership` in that org carrying `r`
    ///   (D-E3-11: role approval needs a REAL membership, not a cosmetic title).
    /// - **`NamedSet`** asks about identity: is this person one of the people staff
    ///   named? Membership in the set is necessary AND sufficient — naming a set is
    ///   itself the authorization, which is why the named need no grant and why
    ///   **staff do not qualify unless named** (staff-in-general is not a person).
    ///   §4.5 pairs this with a view of the request's accounts for its lifetime, so
    ///   a named approver can see what they are signing.
    ///
    /// `OneAuthorizedActor` qualifies nobody: it is inert (the one authorized actor
    /// is the poster, and four-eyes bars them), which is why `@account/approval`
    /// refuses to store it (A11).
    fn authorize_approver(&self, approver: ObjId, account: &Account, policy: &ApprovalPolicy) -> bool {
        // Liveness first, for both policies: a recorded approval is re-qualified at
        // post time, and a destroyed approver qualifies for nothing. The `Threshold`
        // arm fails closed anyway (a dead object holds no capability and no live
        // membership), but `NamedSet` asks only about identity — and an id in a
        // stored set outlives the person, so a destroyed named approver's recorded
        // approval would keep counting forever (A5).
        if !approver_live(self.w, approver) {
            return false;
        }
        match policy {
            ApprovalPolicy::OneAuthorizedActor => false,
            ApprovalPolicy::NamedSet { actors, .. } => actors.contains(&approver),
            ApprovalPolicy::Threshold { from_role, .. } => {
                let base = is_staff(self.w, approver)
                    || account.scope.is_some_and(|s| {
                        scope_coherent(self.w, Some(s), &account.owner)
                            && holds_capability(self.w, approver, "economy.account.approve", s)
                    });
                if !base {
                    return false;
                }
                let Some(role) = from_role else { return true };
                let Some(scope) = account.scope else { return false };
                let Some(store) = self.w.module_state::<mush_org::OrgStore>() else { return false };
                let state = store.state();
                let Some(sc) = state.scope(scope) else { return false };
                state
                    .membership(sc.org, approver)
                    .is_some_and(|m| m.status == MembershipStatus::Active && m.roles.contains(role))
            }
        }
    }
}

impl WorldAuth<'_> {
    /// The capability path: a live, coherent compartment plus the capability on it.
    fn scoped_capability(&self, scope: Option<OrgScopeId>, owner: &PrincipalId, capability: &str, actor: ObjId) -> bool {
        let Some(scope) = scope else { return false };
        scope_coherent(self.w, Some(scope), owner) && holds_capability(self.w, actor, capability, scope)
    }
}

// ---- instrument + account creation (non-money writes) ---------------------

/// Register (or replace) an instrument definition. Not money — a normal write.
/// The caller has validated the scale ≤ [`crate::MAX_SCALE`].
pub fn register_instrument(ctx: &mut Ctx<'_>, def: InstrumentDef) {
    persist_instrument(ctx.out, &def);
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.snapshot.put_instrument(def);
    }
}

/// Open an account (zero balance). Not money — a normal write. The `id` must come
/// from [`EconomyStore::alloc_account`].
pub fn open_account(ctx: &mut Ctx<'_>, account: Account) {
    let meta = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        store.snapshot.put_account(account.clone());
        store.meta_doc()
    };
    persist_account(ctx.out, &account);
    persist_meta(ctx.out, meta);
}

/// Create a vault (empty). Not money and not title — a normal record write, like
/// [`open_account`]. The `id` must come from [`EconomyStore::alloc_vault`], and the
/// caller has already proved the backing is usable and unclaimed.
pub fn create_vault(ctx: &mut Ctx<'_>, vault: Vault) {
    let meta = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        store.snapshot.put_vault(vault.clone());
        store.meta_doc()
    };
    persist_vault(ctx.out, &vault);
    persist_meta(ctx.out, meta);
}

/// Staff recovery: set (or clear) an asset's doc-authority `frozen` latch. **Not
/// journaled** — the latch is deliberately outside every `AssetEffect` (D-E3-1), so
/// clearing it is a doc write and a lost doc rebuilds *frozen*.
///
/// It deliberately does **not** bump `revision`: `revision` is a journal-authority
/// field, and moving it here would make the doc disagree with the journal at its own
/// claimed sequence — the next boot would read the freeze itself as drift and latch
/// the whole-economy freeze (the C2 regression).
pub fn set_asset_frozen(ctx: &mut Ctx<'_>, object: ObjId, frozen: bool, reason: Option<SmolStr>) -> bool {
    let doc = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return false };
        let Some(a) = store.snapshot.asset_mut(object) else { return false };
        a.frozen = frozen;
        a.frozen_reason = if frozen { reason } else { None };
        a.clone()
    };
    persist_asset(ctx.out, &doc);
    true
}

/// Set an account's doc-authority `status` (E3b2: `@account/close`, staff
/// `@account/unfreeze`). **Not journaled**, and deliberately **does not bump
/// `revision`** — the same discipline as [`set_asset_frozen`]: `revision` moves with
/// the journal (`apply` bumps it on every touched account), so nudging it from a doc
/// write would make the doc claim a state its own `last_journal_seq` never produced.
/// Returns `false` if the account is unknown.
pub fn set_account_status(ctx: &mut Ctx<'_>, id: AccountId, status: crate::record::AccountStatus) -> bool {
    let doc = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return false };
        let Some(a) = store.snapshot.account_mut(id) else { return false };
        a.status = status;
        a.clone()
    };
    persist_account(ctx.out, &doc);
    true
}

/// Set (or clear) an account's per-account [`crate::ApprovalRule`] (staff,
/// `@account/approval`). A doc write like [`set_account_status`], and for the same
/// reason it leaves `revision` alone.
///
/// The rule is **not** part of the plan hash: an approval authorizes an intent, so
/// changing the rule never invalidates a request already collected — but the rule is
/// re-read live at every post, so raising it does take effect immediately.
pub fn set_account_approval(ctx: &mut Ctx<'_>, id: AccountId, rule: Option<crate::record::ApprovalRule>) -> bool {
    let doc = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return false };
        let Some(a) = store.snapshot.account_mut(id) else { return false };
        a.approval = rule;
        a.clone()
    };
    persist_account(ctx.out, &doc);
    true
}

/// Staff recovery: set (or clear) the **whole-economy** freeze latch, and persist
/// it (the allocator `meta` doc's `frozen` field).
///
/// The latch is durable precisely so that clearing it is a decision someone makes:
/// the boot that detects drift freezes, heals the drifted projections to the
/// journal, and persists them, so nothing in the docs would ever say "frozen" again
/// — only this write does. Per-account / per-asset / per-vault unfreezing stays
/// separate (`@account/unfreeze`, `@asset/unfreeze`, `@vault/unfreeze`): clearing
/// the economy-wide latch must not silently clear the records that named which
/// resources actually need attention.
pub fn set_economy_frozen(ctx: &mut Ctx<'_>, frozen: bool) -> bool {
    let meta = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return false };
        store.frozen = frozen;
        store.meta_doc()
    };
    persist_meta(ctx.out, meta);
    true
}

/// Staff: set a vault's doc-authority status. Not journaled (same split).
pub fn set_vault_status(ctx: &mut Ctx<'_>, id: VaultId, status: crate::record::VaultStatus) -> bool {
    let doc = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return false };
        let Some(v) = store.snapshot.vault_mut(id) else { return false };
        v.status = status;
        v.revision += 1;
        v.clone()
    };
    persist_vault(ctx.out, &doc);
    true
}

// ---- the money seam -------------------------------------------------------

/// The result of a post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostOutcome {
    /// A new transaction posted (its confirmation is strict-durable-gated).
    Posted(TransactionId),
    /// The idempotency key already resolved to this transaction (no new post).
    AlreadyPosted(TransactionId),
    /// A per-account [`crate::ApprovalRule`] fired: **nothing posted**, and this
    /// request must be satisfied before the same plan will. E3b2 mints it on the
    /// first submission and re-serves the same id on every later one (one live
    /// request per hash, A1), so re-submitting is safe and idempotent.
    NeedsApproval(ApprovalRequestId),
}

/// Post a validated plan: the whole money **and title and custody** path. Refuses
/// when the economy is frozen, validates live, performs the custody moves through
/// the resolved providers, applies, writes the atomic bundle, and gates the
/// confirmation on the strict durable ack.
///
/// **`post` is the LAST fallible call a handler may make** (§4.2 step 6, the
/// `mush_org::submit` contract). A handler that returns `Err` discards its deferred
/// persists but NOT its world mutations, so anything fallible after a successful
/// `post` can leave the world ahead of the journal. Parse and resolve everything
/// before calling it, and never `?` afterwards.
///
/// Ordering inside (§4.2): validate → custody moves (driven **only** by
/// `Prepared.custody_effects`, so an `AlreadyPosted` retry performs zero world
/// mutation) → apply + index → queue the bundle → strict gate. A provider `Err`
/// returns before the store is touched.
pub fn post(ctx: &mut Ctx<'_>, plan: &EconomyPlan) -> Result<PostOutcome, EconomyError> {
    let (outcome, providers) = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        if store.frozen {
            return Err(EconomyError::EconomyFrozen);
        }
        let providers = store.providers();
        let auth = WorldAuth { w: ctx.w };
        let custody = WorldCustody { w: ctx.w, providers: providers.clone() };
        let env = ValidateEnv { auth: &auth, custody: &custody, now: ctx.w.now() };
        (validate(plan, store.snapshot(), &env), providers)
    };
    finish(ctx, outcome, &providers, &plan.actor)
}

/// Reverse transaction `target`: fetch the real original from the journal-derived
/// index and validate its inverse (so the `reverses` back-reference is engine-set,
/// never forgeable). Strict-durable-gated like any money move.
pub fn reverse(
    ctx: &mut Ctx<'_>,
    target: TransactionId,
    actor: PrincipalId,
    cause: EconomicCause,
) -> Result<PostOutcome, EconomyError> {
    let (outcome, providers) = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        if store.frozen {
            return Err(EconomyError::EconomyFrozen);
        }
        let original = store.transaction(target).ok_or(EconomyError::NoSuchTransaction(target))?.clone();
        let auth = WorldAuth { w: ctx.w };
        (plan_reversal(&original, actor.clone(), cause, store.snapshot(), &auth), store.providers())
    };
    finish(ctx, outcome, &providers, &actor)
}

/// Run the prepared custody moves through their resolved providers — §4.2 step 2,
/// **before** the store advances, so a refusal leaves RAM, the journal, and the
/// deferred persists all untouched.
///
/// Every backing is resolved (and every claim checked) in a read-only first pass, so
/// a mis-routed op never reaches a mutation. A plan carrying several custody ops
/// applies them in order: validation already proved standing and destination for all
/// of them against one snapshot, but a provider that nonetheless refuses the *k*-th
/// move leaves the first *k−1* applied — commands therefore build at most one.
fn run_custody_effects(
    ctx: &mut Ctx<'_>,
    prepared: &Prepared,
    providers: &crate::custody::CustodyTable,
) -> Result<(), EconomyError> {
    if prepared.custody_effects.is_empty() {
        return Ok(());
    }
    let PrincipalId::Object(actor) = prepared.actor else {
        return Err(EconomyError::Custody("only an object principal can move custody".into()));
    };
    let mut moves: Vec<(crate::record::CustodyRef, ObjId, CustodyDirection, ObjId)> = Vec::new();
    {
        let store = ctx.w.module_state::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        for e in &prepared.custody_effects {
            let vault = store.snapshot().vault(e.vault).ok_or(EconomyError::NoSuchVault(e.vault))?;
            moves.push((vault.backing.clone(), e.object, e.direction, e.to));
        }
    }
    // Read-only routing pass: no provider is invoked until every op has exactly one.
    for (backing, obj, _, _) in &moves {
        crate::custody::resolve(providers, ctx.w, backing, Some(*obj))
            .map_err(|e: CustodyError| EconomyError::Custody(e.to_string()))?;
    }
    for (backing, obj, direction, to) in moves {
        let provider = {
            let p = crate::custody::resolve(providers, ctx.w, &backing, Some(obj))
                .map_err(|e: CustodyError| EconomyError::Custody(e.to_string()))?;
            std::sync::Arc::clone(p)
        };
        let done = match direction {
            CustodyDirection::Deposit => provider.move_into(ctx, actor, obj, &backing),
            CustodyDirection::Withdraw => provider.move_out(ctx, actor, obj, &backing, to),
        };
        done.map_err(|e| EconomyError::Custody(e.to_string()))?;
    }
    Ok(())
}

/// The **module-layer** custody arity cap (§4.2 / E3b1's finding). `validate`
/// happily proves a plan carrying several custody ops against one snapshot, but
/// [`run_custody_effects`] applies them one provider call at a time: a provider that
/// refuses the *k*-th move leaves the first *k−1* applied in the world with nothing
/// journaled. Until a two-phase custody protocol exists (not E3), the module refuses
/// the shape outright rather than trusting every caller to build only one.
const MAX_CUSTODY_EFFECTS_AT_MODULE: usize = 1;

/// Serve, or mint, the [`ApprovalRequest`] a `NeedsApproval` plan needs (§4.6).
///
/// **No money moves here**, so the confirmation rides the ordinary (non-strict)
/// durable gate: the worst case for a dropped request doc is that the requester
/// re-submits and gets a fresh id, never a lost or double post.
fn mint_or_serve_approval(
    ctx: &mut Ctx<'_>,
    need: &ApprovalNeed,
    requester: &PrincipalId,
) -> Result<PostOutcome, EconomyError> {
    let now = ctx.w.now();
    // A1: exactly one LIVE request per hash. An existing, unexpired one is simply
    // re-served — this is what makes re-submitting the same plan idempotent instead
    // of minting a second request that could collect approvals in parallel.
    let (live, stale) = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        match store.snapshot().live_approval_for_hash(&need.plan_hash) {
            Some(r) if now < r.expires_at => (Some(r.id), None),
            Some(r) => (None, Some(r.clone())),
            None => (None, None),
        }
    };
    if let Some(id) = live {
        ctx.gate_on_durable_ack();
        return Ok(PostOutcome::NeedsApproval(id));
    }

    // G1 caps FIRST — before the stale-request retire below, which mutates RAM and
    // queues a write. A handler that returns `Err` discards its deferred persists
    // but NOT its world mutations (§4.2), so retiring first would leave the doc
    // saying `Pending` and RAM saying `Expired` whenever a cap then refuses: the
    // by-hash index would go quiet in RAM while the durable record still claimed a
    // live request. Nothing between here and the mint reads the retired request.
    {
        let store = ctx.w.module_state::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        let pending = || {
            store
                .snapshot()
                .approvals()
                .filter(move |r| r.status == ApprovalStatus::Pending && now < r.expires_at)
        };
        if pending().filter(|r| r.requested_by == *requester).count() >= crate::MAX_PENDING_PER_REQUESTER {
            return Err(EconomyError::TooLarge("too many pending approval requests for this requester"));
        }
        for account in &need.accounts {
            if pending().filter(|r| r.accounts.contains(account)).count() >= crate::MAX_PENDING_PER_ACCOUNT {
                return Err(EconomyError::TooLarge("too many pending approval requests for this account"));
            }
        }
    }

    // A request that aged out still holds the by-hash index (its status is only
    // recomputed against the clock at boot). Retire it explicitly, or the fresh
    // request would be minted and then ignored by `validate`, which reads the index.
    if let Some(mut old) = stale {
        old.status = ApprovalStatus::Expired;
        old.revision += 1;
        put_approval(ctx, old);
    }

    let request = {
        let store = ctx.w.module_state_mut::<EconomyStore>().expect("economy store");
        ApprovalRequest {
            id: store.alloc_approval(),
            plan_hash: need.plan_hash,
            accounts: need.accounts.clone(),
            requested_by: requester.clone(),
            approvals: Vec::new(),
            decisions: Vec::new(),
            created_at: now,
            expires_at: now.saturating_add(APPROVAL_TTL_SECS),
            status: ApprovalStatus::Pending,
            revision: 1,
        }
    };
    let id = request.id;
    put_approval(ctx, request);
    let meta = ctx.w.module_state::<EconomyStore>().expect("economy store").meta_doc();
    persist_meta(ctx.out, meta);
    ctx.gate_on_durable_ack();
    Ok(PostOutcome::NeedsApproval(id))
}

/// Install an approval request into the live snapshot and queue its doc. The
/// caller owns the state transition (the command layer records decisions; `post`
/// mints and expires) — this is the single write path for both.
pub(crate) fn put_approval(ctx: &mut Ctx<'_>, request: ApprovalRequest) {
    {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        store.snapshot.put_approval(request.clone());
    }
    persist_approval(ctx.out, &request);
}

/// Common tail: turn a [`PlanResult`] into a durable-gated outcome or an error.
fn finish(
    ctx: &mut Ctx<'_>,
    outcome: PlanResult,
    providers: &crate::custody::CustodyTable,
    requester: &PrincipalId,
) -> Result<PostOutcome, EconomyError> {
    match outcome {
        PlanResult::AlreadyPosted(id) => {
            // R-R2: an idempotency HIT during the ORIGINAL's durability window must
            // not confirm before the original is durable — else a retry (a
            // deterministic reversal key, or a ruleset's stable key) could deliver
            // "already posted" for a transaction whose sole durable write then drops,
            // so it never happened at reboot (balances are journal-derived). Gating
            // rides the E0 retained-seal re-emit: a HIT's durable flush carries any
            // un-acked sealed journal chunk (the original's), so the confirmation
            // waits for it to ack. (An original still in the open chunk is covered by
            // its own in-flight strict gate; that residual sub-window closes with the
            // deferred durability checkpoint, R-D1.)
            ctx.gate_on_durable_ack_strict();
            Ok(PostOutcome::AlreadyPosted(id))
        }
        // The plan is well-formed and authorized but unapproved: mint (or re-serve)
        // its request. Nothing posts either way — this is not a refusal, it is the
        // §4.6 escalation, and the requester's next submission of the SAME plan
        // posts once the approvals are live-qualifying.
        PlanResult::NeedsApproval(need) => mint_or_serve_approval(ctx, &need, requester),
        PlanResult::Refused(e) => Err(e),
        PlanResult::Post(prepared) => {
            // The module-layer custody arity cap: refuse a shape that could
            // half-apply, BEFORE any provider is called.
            if prepared.custody_effects.len() > MAX_CUSTODY_EFFECTS_AT_MODULE {
                return Err(EconomyError::TooLarge("one custody op per plan at the module layer"));
            }
            // §4.2 step 2: the world moves BEFORE the store advances, and only from
            // the prepared effects (never the plan's ops — see `AlreadyPosted` above,
            // which performs zero world mutation).
            run_custody_effects(ctx, &prepared, providers)?;
            let now = ctx.w.now();
            let txid = commit_prepared(ctx, &prepared, now);
            // Invariant 9 / R-R1: the "posted" confirmation must never outrun the
            // durable ack — strict so a dropped write suppresses it entirely.
            ctx.gate_on_durable_ack_strict();
            Ok(PostOutcome::Posted(txid))
        }
    }
}

/// Allocate ids, apply the prepared post to the live snapshot, and queue the whole
/// bundle (journal append + touched account projections + allocator meta) on one
/// write batch. The store mutation happens in a scope that ends before `ctx.out` is
/// used (the shipped module discipline).
fn commit_prepared(ctx: &mut Ctx<'_>, prepared: &Prepared, now: i64) -> TransactionId {
    let (txid, journal_op, account_docs, asset_docs, approval_doc, meta) = {
        let store = ctx.w.module_state_mut::<EconomyStore>().expect("economy store");
        let txid = store.alloc_transaction();
        // Peek the seq the journal will assign, so the transaction carries it before
        // the append (which then returns the same seq).
        let seq = store.journal.next_seq(JOURNAL_LOG);
        let (snapshot2, tx) = apply(store.snapshot(), prepared, txid, seq, now);
        let value = serde_json::to_value(&tx).expect("transaction serializes");
        let (_seq2, op) = store.journal.append(JOURNAL_LOG, value);
        debug_assert_eq!(seq, _seq2, "journal seq peeked == appended");
        store.snapshot = snapshot2;
        store.index_transaction(tx);
        let account_docs: Vec<Account> = prepared
            .touched_accounts()
            .filter_map(|id| store.snapshot().account(id).cloned())
            .collect();
        // Every touched title projection rides the same bundle (D-E3-1: the doc is a
        // projection, the effect in the journal is the authority).
        let asset_docs: Vec<EconomicAsset> = prepared
            .asset_effects
            .iter()
            .filter_map(|e| store.snapshot().asset(e.object).cloned())
            .collect();
        // The consumed request rides the SAME bundle as the transaction that spent
        // it (`apply` already flipped it to `Consumed { transaction }`), so a doc
        // drop is healed at boot by id — never by hash (C3).
        let approval_doc = prepared
            .consumed_approval
            .and_then(|id| store.snapshot().approval(id).cloned());
        (txid, op, account_docs, asset_docs, approval_doc, store.meta_doc())
    };
    persist_journal(ctx.out, journal_op);
    for a in &account_docs {
        persist_account(ctx.out, a);
    }
    for a in &asset_docs {
        persist_asset(ctx.out, a);
    }
    if let Some(r) = &approval_doc {
        persist_approval(ctx.out, r);
    }
    persist_meta(ctx.out, meta);
    txid
}
