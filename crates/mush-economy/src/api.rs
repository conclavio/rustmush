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
    Ctx, Deferred, MAX_CONTAINMENT_DEPTH, ModuleOp, ModuleWrite, ObjId, Outbox, Powers,
    PreparedLedgerAppend, Role, World, perm,
};
use mush_org::{
    ApprovalPolicy, AuthRequest, CapabilityId, MembershipStatus, OrgScopeId, OrgStatus, PrincipalId,
    ScopeStatus, authorize_in_world,
};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::custody::{CustodyError, WorldCustody};
use crate::engine::{
    APPROVAL_TTL_SECS, AuthEval, ValidateEnv, apply, plan_reversal, validate,
};
use crate::ids::{
    AccountId, ApprovalRequestId, CadenceId, HoldId, InstrumentId, ObligationId, TransactionId,
    VaultId,
};
use crate::module::{EconomyStore, JOURNAL_LOG, NS};
use crate::plan::{
    ApprovalNeed, AssetOp, CustodyOp, EconomyError, EconomyPlan, HoldOp, PlanResult, Prepared,
};
use crate::record::{
    Account, ApprovalRequest, ApprovalStatus, CustodyDirection, EconomicAsset, EconomicCause,
    EconomicTransaction, Hold, InstrumentDef, Vault,
};
use crate::snapshot::EconomySnapshot;

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
/// `hold:{id}` — a **journal-derived projection** (E4b1, D-E4-1). A hold carries no
/// doc-authority latch at all, so a missing (or unreadable) doc is rebuilt silently
/// from the journal; only a doc that DISAGREES with the journal at its own claimed
/// sequence, or one ahead of it, freezes the economy.
#[must_use]
pub fn hold_key(id: HoldId) -> SmolStr {
    SmolStr::new(format!("hold:{}", id.0))
}
/// `obligation:{id}` — a **container** doc (D-E4-5): obligation *state* is doc
/// authority, its money is the journal. A corrupt one fails the boot.
#[must_use]
pub fn obligation_key(id: ObligationId) -> SmolStr {
    SmolStr::new(format!("obligation:{}", id.0))
}
/// `cadence:{id}` — a **container** doc (D-E4-2): the economy-owned campaign clock.
#[must_use]
pub fn cadence_key(id: CadenceId) -> SmolStr {
    SmolStr::new(format!("cadence:{}", id.0))
}
/// `contract:{id}` — a **container** doc (D-E4-5, §5): contract *state* is doc
/// authority, its money is the journal. A corrupt one fails the boot, because
/// proceeding without it would classify a live agreement — and the escrow standing
/// behind it — as absent.
#[must_use]
pub fn contract_key(id: crate::ids::ContractId) -> SmolStr {
    SmolStr::new(format!("contract:{}", id.0))
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
pub(crate) fn persist_hold(out: &mut Outbox, h: &crate::record::Hold) {
    push_op(out, ModuleWrite::JsonSet {
        key: hold_key(h.id),
        value: serde_json::to_value(h).expect("hold serializes"),
    });
}
pub(crate) fn persist_obligation(out: &mut Outbox, o: &crate::obligation::Obligation) {
    push_op(out, ModuleWrite::JsonSet {
        key: obligation_key(o.id),
        value: serde_json::to_value(o).expect("obligation serializes"),
    });
}
pub(crate) fn persist_cadence(out: &mut Outbox, c: &crate::clock::CadenceClock) {
    push_op(out, ModuleWrite::JsonSet {
        key: cadence_key(c.id),
        value: serde_json::to_value(c).expect("cadence serializes"),
    });
}
pub(crate) fn persist_contract(out: &mut Outbox, c: &crate::contract::EconomicContract) {
    push_op(out, ModuleWrite::JsonSet {
        key: contract_key(c.id),
        value: serde_json::to_value(c).expect("contract serializes"),
    });
}
pub(crate) fn persist_escalation(out: &mut Outbox, e: &crate::escalation::Escalation) {
    push_op(out, ModuleWrite::JsonSet {
        key: crate::escalation::escalation_key(e.id),
        value: serde_json::to_value(e).expect("escalation serializes"),
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
///
/// `pub(crate)` since the E4b1 verify fold (S1): the sweep's `ObligationAuth` asks
/// the **same** question rather than a second, weaker one of its own.
#[must_use]
pub(crate) fn credit_allowed(w: &World, account: &Account) -> bool {
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

    /// Invariant 25 / E4a's handoff: **`authorize_hold` decides everything about who
    /// may touch a reservation.** The beneficiary captures, the placer (or the
    /// beneficiary, or debit authority on the account, or staff) releases, and the
    /// sweep — and only the sweep — expires.
    ///
    /// Three deliberate absences:
    ///
    /// - **`Hold.holder` is never consulted.** It is a caller-chosen idempotency key,
    ///   an attribution label and nothing more (E4a verify fold, finding 8).
    /// - **Plain debit authority does not capture, and does not release a reservation
    ///   that was made TO someone** (E4b1 verify fold, A1). Releasing hands the money
    ///   back to the account it was reserved on, which takes nothing from anyone —
    ///   *unless* the reservation has a beneficiary, in which case it takes away
    ///   exactly the thing that was promised to them. E4b1 shipped the fallback
    ///   unconditionally, so the payer could destroy their own escrow with their own
    ///   wallet rights and the payee's guarantee was worth nothing. The fallback now
    ///   applies only to a **beneficiary-less** hold; a hold made *to* someone is
    ///   released by that beneficiary (or anyone who acts for them), by its placer, or
    ///   by staff.
    /// - **`Place` is not answered here.** The engine asks it as debit authority on
    ///   the account through [`AuthEval::authorize`] (a reservation is a promise to
    ///   pay); a `Place` reaching this method at all would be a shape the engine does
    ///   not produce, so it fails closed.
    fn authorize_hold(&self, actor: &PrincipalId, hold: &Hold, op: &HoldOp) -> bool {
        // **Escrow is the contract's, and only the contract's** (E4b1 verify fold's
        // E4b2 handoff, §4.8). A reservation whose purpose is a contract's escrow is
        // released by that contract's own refund path, captured by its own
        // settlement, and touched by nothing else — not by its placer, not by the
        // beneficiary, not by staff, and not by the sweep. `ContractAuth` is the only
        // authority that answers `true` for one, and it is reachable only through
        // `post_with(.., PostAuth::Injected(..))` from inside this crate.
        //
        // Without this the placer clause below was an open door: the accepting party
        // places their own escrow, so they would have been able to release it the
        // instant afterwards and leave a live contract standing on nothing.
        if matches!(hold.purpose, crate::record::HoldPurpose::Escrow { .. }) {
            return false;
        }
        match op {
            HoldOp::Place { .. } => false,
            HoldOp::Capture { .. } => {
                let Some(beneficiary) = &hold.beneficiary else {
                    // A reservation for nobody in particular has no one who may spend
                    // it — it can only be released or aged out.
                    return false;
                };
                self.acts_for(actor, beneficiary)
            }
            HoldOp::Release { .. } => {
                if *actor == hold.placed_by {
                    return true;
                }
                // A promise made TO someone is theirs to give back, not the payer's
                // to take back (A1): with a beneficiary, the beneficiary (or whoever
                // acts for them, staff included) is the only other releaser.
                if let Some(beneficiary) = &hold.beneficiary {
                    return self.acts_for(actor, beneficiary);
                }
                let Some(store) = self.w.module_state::<EconomyStore>() else { return false };
                let Some(account) = store.snapshot().account(hold.account) else { return false };
                let Some(probe) = hold.amount.checked_neg() else { return false };
                self.authorize(actor, account, probe)
            }
            // Expiry is the scheduler's act, not a player verb: only the sweep's own
            // system principal, and the engine additionally requires the sweep cause.
            HoldOp::Expire { .. } => *actor == sweep_principal(),
        }
    }

    /// Who currently qualifies as an attestor (E4b2). Re-run **live** at settlement,
    /// exactly like [`AuthEval::authorize_approver`] (E3's A5 rule).
    ///
    /// E4b1 left this at the trait default (`false`), which was invisible until
    /// contracts went live and then would have been a quiet money bug in the wrong
    /// direction: a `Capability` or `Staff` attestation, which `reverify` re-qualifies
    /// through this method, could never re-qualify, so a contract satisfied by one
    /// could never settle at all.
    ///
    /// - `Party(p)`: the actor **is** `p` when `p` is a person, and **acts for** `p`
    ///   when `p` is an organization (E4b2 verify fold, A3). The identity rule reads
    ///   well — "somebody else's word is not theirs" — and it stays, for people. But
    ///   `p` is a `PrincipalId`, and an **organization never types a command**: a
    ///   contract attested by `party:org:Ironhold` could be satisfied by nobody at
    ///   all, so its escrow sat until the deadline with the org's own officers locked
    ///   out. For that case the question is the same one `may_view_hold` and every
    ///   capture ask (E4b1's A2). The *record* still names the party, not the officer
    ///   (`contract_attest`), so a party attestation still stands once given.
    /// - `Capability { scope, capability }`: the capability, live, on a live
    ///   compartment — the ordinary org question. A capability set with no scope
    ///   qualifies nobody (there is no compartment to hold it on).
    /// - `Staff`: the staff predicate the whole crate uses.
    /// - `Canon { .. }`: never a principal. A canon attestation is recorded by E4c's
    ///   observer from a canon event, and `reverify` matches it on kind and source
    ///   without asking this method at all.
    fn authorize_attestor(&self, actor: &PrincipalId, set: &crate::AttestorSet) -> bool {
        match set {
            crate::AttestorSet::Party(p) => match p {
                // A person's word is their own: naming a party means somebody else's
                // assertion is not theirs, staff included (staff have
                // `@contract/resolve` for what they legitimately need here).
                PrincipalId::Object(_) => actor == p,
                // An organization types nothing. Whoever may spend its money speaks
                // for it — the same predicate `may_view_hold` and every capture ask.
                PrincipalId::Organization(_) => actor == p || self.acts_for(actor, p),
                PrincipalId::System(_) => false,
            },
            crate::AttestorSet::Capability { scope, capability } => {
                let (PrincipalId::Object(a), Some(scope)) = (actor, scope) else { return false };
                let req = AuthRequest {
                    actor: *a,
                    capability: capability.clone(),
                    scope: *scope,
                    now: self.w.now(),
                    amount: None,
                };
                authorize_in_world(self.w, &req).is_allow()
            }
            crate::AttestorSet::Staff => {
                matches!(actor, PrincipalId::Object(o) if is_staff(self.w, *o))
            }
            crate::AttestorSet::Canon { .. } => false,
        }
    }
}

impl WorldAuth<'_> {
    /// The capability path: a live, coherent compartment plus the capability on it.
    fn scoped_capability(&self, scope: Option<OrgScopeId>, owner: &PrincipalId, capability: &str, actor: ObjId) -> bool {
        let Some(scope) = scope else { return false };
        scope_coherent(self.w, Some(scope), owner) && holds_capability(self.w, actor, capability, scope)
    }

    /// Whether `actor` acts **for** `who` — the beneficiary question a capture asks,
    /// and (E4b1 verify fold, A2) the same question `may_view_hold` asks, so that
    /// seeing a reservation and spending it can never come apart.
    ///
    /// An object principal is acted for by itself, by anyone who MUSH-controls it,
    /// or by staff. An organization is acted for by a holder of the spending
    /// capability on its root compartment (`economy.account.post` — the E3
    /// vocabulary's spending entry; no new capability is minted, because "may spend
    /// the org's money" is exactly the right to turn its escrow into a payment) or by
    /// staff.
    ///
    /// A **system** principal is acted for by **nobody, staff included** (A3). The
    /// `System` arm sits above the staff short-circuit deliberately: a `System`
    /// beneficiary is an internal settlement authority ([`crate::ContractAuth`]'s,
    /// and E4b2's), and a staffer who could capture one from a command would be
    /// spending a contract's escrow by hand, outside the settlement that is supposed
    /// to be the only thing that can. Staff have `@econ/mint` and `@econ/reverse` for
    /// what they legitimately need here.
    pub(crate) fn acts_for(&self, actor: &PrincipalId, who: &PrincipalId) -> bool {
        let PrincipalId::Object(a) = actor else { return false };
        if matches!(who, PrincipalId::System(_)) {
            return false;
        }
        if is_staff(self.w, *a) {
            return true;
        }
        match who {
            PrincipalId::Object(o) => *a == *o || mush_core::perm::controls(self.w, *a, *o),
            PrincipalId::Organization(org) => {
                let Some(store) = self.w.module_state::<mush_org::OrgStore>() else { return false };
                let Some(root) = store.state().org(*org).map(|o| o.root_scope) else { return false };
                scope_coherent(self.w, Some(root), who)
                    && holds_capability(self.w, *a, "economy.account.post", root)
            }
            PrincipalId::System(_) => false,
        }
    }
}

/// The system principal every economy **sweep** post is attributed to (§4.3). It is
/// not a faucet and owns no account: `WorldAuth` grants it exactly one thing — the
/// right to age a hold out past its own deadline — and the engine independently
/// requires the `Internal { kind: "hold.expire" }` cause for that op (invariant 24).
///
/// Nothing constructs this from player input, and the E4b1 verify fold made that
/// exact (A3, S9):
///
/// - No command builds a plan whose `actor` is a system principal, and this
///   constructor is now `pub(crate)`, so no other crate can name the sweep either.
/// - `parse_principal` does accept `sys:<name>` — `@account/system` and `@econ/mint`
///   name one, and so may `@hold/place for=`. Every such verb is **staff-only** and
///   resolves the name against a **registered** system account first, so a player
///   cannot conjure a system beneficiary and staff cannot conjure one that does not
///   already exist.
/// - Nobody acts for a `System` principal, staff included ([`WorldAuth::acts_for`]),
///   so a reservation held for one is capturable only through an internal settlement
///   authority — never from a command.
#[must_use]
pub(crate) fn sweep_principal() -> PrincipalId {
    PrincipalId::System(
        mush_org::SystemPrincipalId::new("economy.sweep").expect("valid system principal"),
    )
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
    post_with(ctx, plan, PostAuth::World)
}

/// Which [`AuthEval`] a post is validated against (§4.1).
///
/// [`WorldAuth`] borrows the world, and a caller holding `&mut Ctx` cannot build one
/// to hand in — so the *choice* is passed instead and the bridge is constructed
/// inside [`post_with`], where the borrow lives exactly as long as validation does.
pub(crate) enum PostAuth<'a> {
    /// The live org bridge — every player-facing post.
    World,
    /// An injected authority: the sweep's per-obligation one, and (E4b2)
    /// [`crate::ContractAuth`], whose acceptances ARE the settlement authority.
    Injected(&'a dyn AuthEval),
}

/// [`post`] with an explicit authority (§4.1). `post` is the only public entry;
/// the sweep and (E4b2) settlement reach the same validate → custody → install
/// sequence through here, so an internal post is never a second, weaker code path.
pub(crate) fn post_with(
    ctx: &mut Ctx<'_>,
    plan: &EconomyPlan,
    auth: PostAuth<'_>,
) -> Result<PostOutcome, EconomyError> {
    post_with_companions(ctx, plan, auth, Vec::new())
}

/// [`post_with`], plus **companion container docs written in the post's own batch**
/// (E4b2 verify fold, B2).
///
/// The problem it solves is `@contract/accept`. The escrow placement and the
/// acceptance are two facts about one act, and they were two writes: the post's
/// journal chunk and hold/account projections, then — separately, afterwards — the
/// contract doc carrying `escrow: [id]` and the acceptance. Lose the second and the
/// hold survives replay (it is journal authority) while the record that names it does
/// not: the payer's money is reserved to `HoldPurpose::Escrow { contract }`, which
/// `WorldAuth` refuses every op on, for a contract whose `escrow` list is empty and
/// whose settlement therefore never reaches it. Nothing anywhere could release it.
///
/// Handing the doc to the post closes that: every companion rides
/// [`install_commit`]'s batch with the journal append, and is installed into the same
/// post-image snapshot, so the two facts are durable together or neither is.
pub(crate) fn post_with_companions(
    ctx: &mut Ctx<'_>,
    plan: &EconomyPlan,
    auth: PostAuth<'_>,
    companions: Vec<crate::contract::EconomicContract>,
) -> Result<PostOutcome, EconomyError> {
    let (outcome, providers) = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        if store.frozen {
            return Err(EconomyError::EconomyFrozen);
        }
        let providers = store.providers();
        let world_auth = WorldAuth { w: ctx.w };
        let auth: &dyn AuthEval = match auth {
            PostAuth::World => &world_auth,
            PostAuth::Injected(a) => a,
        };
        let custody = WorldCustody { w: ctx.w, providers: providers.clone() };
        // E4c: the wiring-frozen fact table, read live. An empty table answers
        // `None` for everything, which is unknown, which is unsatisfied — exactly
        // the `NoFacts` behaviour this replaced.
        let facts = crate::facts::WorldFacts { w: ctx.w, providers: store.facts() };
        let env = ValidateEnv {
            auth,
            custody: &custody,
            facts: &facts,
            now: ctx.w.now(),
            ic_now: ctx.w.game_now_unix(),
            next_hold_id: store.peek_next_hold(),
        };
        (validate(plan, store.snapshot(), &env), providers)
    };
    finish(ctx, outcome, &providers, &plan.actor, companions)
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
    finish(ctx, outcome, &providers, &actor, Vec::new())
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
        // B2: the allocator can refuse at the ceiling. A request that cannot be
        // minted is not an approval that is silently skipped — the post is refused,
        // loudly, and nothing moves.
        let Some(id) = store.alloc_approval() else {
            tracing::error!("economy: the approval-request allocator is exhausted; refusing the post");
            return Err(EconomyError::Overflow);
        };
        ApprovalRequest {
            id,
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

/// Everything [`create_system_obligation`] needs that is not the faucet itself.
///
/// A struct rather than eight positional arguments, and deliberately **not** an
/// `Obligation`: the id, the two principals, the counters, the history and
/// `minted_by` are the economy's to fill in, not the ruleset's.
#[derive(Clone, Debug)]
pub struct ObligationSpec {
    /// The faucet's own account (its owner must be the faucet principal).
    pub debit_account: AccountId,
    /// Who is paid each period.
    pub credit_account: AccountId,
    /// How much, per period — bounded by the live [`crate::EconomyConfig`] cap.
    pub amount: Amount,
    /// When and how often.
    pub schedule: crate::clock::EconomicSchedule,
    /// How much slack before delinquency.
    pub grace: crate::obligation::GracePolicy,
    /// Whether a routine success notifies.
    pub notify_on_success: bool,
    /// What created it (a ruleset namespace, normally).
    pub cause: crate::obligation::ObligationCause,
}

/// **The ruleset's mint entry** (E4b1 verify fold, M1): create a recurring payment
/// funded by a registered faucet.
///
/// Every other route to a `System` debit is closed and stays closed —
/// `@obligation/create` refuses the shape, and the sweep's `ObligationAuth` refuses
/// it again at every settlement. A faucet-funded obligation is a **standing mint**:
/// the scheduler will execute it forever, with nobody in the loop, so opening it is
/// not a staff verb (a staffer can be socially engineered, and `@econ/mint` already
/// covers the one-off case). It is a call the ruleset makes in code, under four
/// locks:
///
/// 1. **The faucet must be registered** — it must own the debit account, which only
///    `@account/system` creates. A name nobody registered mints nothing.
/// 2. **The debit account must be that faucet's own**, `Active`, and the credit
///    account must be a live account in the same instrument. No implicit exchange,
///    and no minting into a closed wallet.
/// 3. **The amount must be inside the configured cap**
///    ([`crate::EconomyConfig::mint_cap`]), refused as
///    [`EconomyError::MintCapExceeded`] otherwise.
/// 4. **The cap is re-checked at every settlement**, live, so lowering it in the
///    config binds the next sweep and a record edited above it afterwards is paged
///    rather than paid.
///
/// The record is a container doc like any other obligation; the ordinary durable
/// gate applies, because nothing has moved yet.
pub fn create_system_obligation(
    ctx: &mut Ctx<'_>,
    faucet: &mush_org::SystemPrincipalId,
    spec: ObligationSpec,
) -> Result<ObligationId, EconomyError> {
    let owner = PrincipalId::System(faucet.clone());
    if spec.amount.is_zero() || spec.amount.is_negative() {
        return Err(EconomyError::ZeroLine(0));
    }
    if spec.debit_account == spec.credit_account {
        return Err(EconomyError::ScopeOwnerMismatch);
    }
    let (debtor, creditor) = {
        let store = ctx.w.module_state::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        if store.frozen {
            return Err(EconomyError::EconomyFrozen);
        }
        let snap = store.snapshot();
        let debit =
            snap.account(spec.debit_account).ok_or(EconomyError::NoSuchAccount(spec.debit_account))?;
        let credit = snap
            .account(spec.credit_account)
            .ok_or(EconomyError::NoSuchAccount(spec.credit_account))?;
        // Lock 1 + 2: this account IS the named faucet's, and both sides are live.
        if debit.owner != owner {
            return Err(EconomyError::Unauthorized);
        }
        if debit.status != crate::record::AccountStatus::Active {
            return Err(EconomyError::InactiveAccount(spec.debit_account));
        }
        if credit.status != crate::record::AccountStatus::Active {
            return Err(EconomyError::InactiveAccount(spec.credit_account));
        }
        if debit.instrument != credit.instrument {
            return Err(EconomyError::NotBalanced(debit.instrument.clone()));
        }
        // Lock 3: the configured ceiling, in this instrument's own minor units. An
        // unrepresentable cap refuses — it is not an absent one.
        let cap = store
            .config()
            .mint_cap(snap, &debit.instrument)
            .ok_or(EconomyError::MintCapExceeded(spec.debit_account))?;
        if spec.amount > cap {
            return Err(EconomyError::MintCapExceeded(spec.debit_account));
        }
        (debit.owner.clone(), credit.owner.clone())
    };
    let id = {
        let store = ctx.w.module_state_mut::<EconomyStore>().ok_or(EconomyError::EconomyFrozen)?;
        store.alloc_obligation().ok_or(EconomyError::Overflow)?
    };
    let o = crate::obligation::Obligation {
        id,
        debtor,
        creditor,
        debit_account: spec.debit_account,
        credit_account: spec.credit_account,
        amount: spec.amount,
        schedule: spec.schedule,
        grace: crate::obligation::GracePolicy::new(spec.grace.periods),
        notify_on_success: spec.notify_on_success,
        status: crate::obligation::ObligationStatus::Active,
        cause: spec.cause,
        minted_by: Some(owner),
        periods_settled: 0,
        history: Vec::new(),
        settled_total: 0,
        missed_total: 0,
        forgiven_total: 0,
        revision: 1,
    };
    put_obligation(ctx, o);
    let meta = ctx.w.module_state::<EconomyStore>().map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        persist_meta(ctx.out, meta);
    }
    ctx.gate_on_durable_ack();
    Ok(id)
}

/// Install an obligation into RAM and queue its container doc (E4b1).
///
/// The latch rule (§4.2) is trivially satisfied here and worth stating: an
/// [`crate::Obligation`] carries **no journal-authority field at all**. Its money is
/// the journal — every settled period is an ordinary transaction — while its status,
/// schedule and history are doc authority end to end (D-E4-5), so a status write can
/// never make a doc disagree with the journal at a sequence it claims.
pub(crate) fn put_obligation(ctx: &mut Ctx<'_>, o: crate::obligation::Obligation) {
    let changed = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        // E4c: an event marks a **transition**, not a write. An obligation is
        // re-persisted on every settled period, and an observer that published one
        // canon line per rent payment would be a chronicle nobody could read.
        let before = store.snapshot.obligation(o.id).map(|p| p.status.kind());
        let after = o.status.kind();
        store.snapshot.put_obligation(o.clone());
        before != Some(after)
    };
    persist_obligation(ctx.out, &o);
    if changed {
        crate::observer::emit(ctx, &crate::record::EconomicEvent::ObligationChanged {
            id: o.id,
            status_kind: o.status.kind(),
        });
    }
}

/// Install a cadence clock into RAM and queue its container doc (E4b1, D-E4-2).
pub(crate) fn put_cadence(ctx: &mut Ctx<'_>, c: crate::clock::CadenceClock) {
    {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        store.snapshot.put_cadence(c.clone());
    }
    persist_cadence(ctx.out, &c);
}

/// Install a contract into RAM and queue its container doc (E4b2, §5).
///
/// The latch rule (§4.2) holds here the way it does for an obligation: an
/// [`crate::EconomicContract`] carries **no journal-authority field**. Its money is
/// the journal — the escrow holds, the settlement transaction — while its state,
/// acceptances, attestations and roles are doc authority end to end (D-E4-5), so a
/// state write can never make a doc disagree with the journal at a sequence it
/// claims.
///
/// [`EconomySnapshot::put_contract`] re-derives the custody locks from the record's
/// `DepositInto` conditions as it installs it, so the lock lifts on exactly the
/// state change that is being written and never a moment earlier.
pub(crate) fn put_contract(ctx: &mut Ctx<'_>, c: crate::contract::EconomicContract) {
    let changed = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        // Only a real **state transition** is an event (E4c). A contract doc is
        // rewritten by every builder verb, every acceptance and every failed settle
        // attempt; publishing all of those would make the canon bridge a firehose.
        let before = store.snapshot.contract(c.id).map(|p| p.state.kind());
        let after = c.state.kind();
        store.snapshot.put_contract(c.clone());
        before != Some(after)
    };
    persist_contract(ctx.out, &c);
    if changed {
        // The **landed transaction id** is the event's dedupe key (§6.1): a
        // settlement finished a second time — by the sweep's `AlreadyPosted` retry,
        // or by boot reading the same deterministic key out of the idempotency
        // index — carries the same id, so a publishing observer can tell a repeat
        // from a new settlement. A non-money transition carries `None`.
        let transaction = match &c.state {
            crate::contract::ContractState::Settled { transaction } => Some(*transaction),
            _ => None,
        };
        crate::observer::emit(ctx, &crate::record::EconomicEvent::ContractChanged {
            id: c.id,
            state: c.state.kind(),
            transaction,
        });
    }
}

// ---- title issuance for registered system principals (E4c, §6.1 item 6) ----

/// The one [`AuthEval`] a [`issue_title`] post runs under: it authorizes **exactly
/// one** `AssetOp::Title`, for exactly the object, owner and compartment the caller
/// named, on behalf of exactly the registered issuer — and nothing else at all.
///
/// Every other method keeps the trait's `false` default, so this authority cannot
/// move money, transfer a title, encumber, assign, touch a vault, qualify an
/// approver, or reach a reservation. It is not a staff impersonation: `WorldAuth`
/// would refuse the same plan outright, because a `System` principal is not an
/// object and nobody acts for one.
struct TitleIssuerAuth {
    issuer: PrincipalId,
    object: ObjId,
    owner: PrincipalId,
    scope: Option<OrgScopeId>,
}

impl AuthEval for TitleIssuerAuth {
    /// No money authority whatsoever — a title issuer mints records, never value.
    fn authorize(&self, _actor: &PrincipalId, _account: &Account, _delta: Amount) -> bool {
        false
    }
    fn authorize_asset(&self, actor: &PrincipalId, asset: Option<&EconomicAsset>, op: &AssetOp) -> bool {
        asset.is_none()
            && *actor == self.issuer
            && matches!(op, AssetOp::Title { object, owner, scope, .. }
                if *object == self.object && *owner == self.owner && *scope == self.scope)
    }
}

/// Mint a title record for `object` on behalf of a **registered** system principal
/// (D-E3-13, fed back by E5's 08d §7 item 4).
///
/// E3 made creating a title staff-only (invariant 20) because the alternative was a
/// capability any org officer could be granted, and title is the thing money buys.
/// But a ruleset that *produces* things — E5's `sys:wod.housing` handing a character
/// the flat they just bought, E7's production output — cannot ask a staffer to type
/// `@asset/title` for every one of them. The resolution is the **frozen-table**
/// pattern this crate already uses for custody providers and the capability
/// vocabulary: an issuer is part of server composition
/// ([`crate::EconomyModule::with_title_issuers`]), not something a command, a
/// document or a grant can add to. An unregistered issuer is refused here, before
/// anything is built, and would be refused again at the engine seam.
///
/// The effect is an ordinary journaled `Created` [`crate::AssetEffect`] with cause
/// `Internal { kind: "title.issue", id: object }` and the deterministic idempotency
/// key `title.issue/{object}`, so two calls for one object resolve to the same
/// transaction rather than racing the `AssetExists` check. The **transaction's actor
/// is the issuer**, so the journal says who minted it forever.
///
/// # Errors
///
/// [`EconomyError::Unauthorized`] for an unregistered issuer or an owner/scope pair
/// the compartment rules refuse; every other refusal is the ordinary posting error.
pub fn issue_title(
    ctx: &mut Ctx<'_>,
    object: ObjId,
    owner: PrincipalId,
    scope: Option<OrgScopeId>,
    issuer: mush_org::SystemPrincipalId,
) -> Result<PostOutcome, EconomyError> {
    // E4c verify fold, E6: **the object has to exist.** The staff path reaches
    // `@asset/title` through `asset_target`, which resolves a live world object; this
    // one took an `ObjId` from its caller and minted title for whatever number it was
    // handed. A title record on a destroyed or never-created object is an asset the
    // economy will carry forever, that no custody provider can locate, that
    // `@asset/show` renders as a ghost, and that boot's reconciliation pass must
    // freeze. Refuse it where it is cheap.
    if !approver_live(ctx.w, object) {
        tracing::warn!(
            issuer = issuer.as_str(),
            object = object.0,
            "economy: a title issuer named an object that is not a live world object; refused"
        );
        return Err(EconomyError::NoSuchAsset(object));
    }
    let registered = ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|s| s.is_title_issuer(&issuer));
    if !registered {
        tracing::warn!(
            issuer = issuer.as_str(),
            object = object.0,
            "economy: an unregistered system principal tried to issue title; refused"
        );
        return Err(EconomyError::Unauthorized);
    }
    // Invariant 18's world half, which the injected authority cannot ask (it holds
    // no `&World`): a scoped title must name the compartment's own organization.
    if !scope_coherent(ctx.w, scope, &owner) {
        return Err(EconomyError::ScopeOwnerMismatch);
    }
    // E4c verify fold, E7: the key carries the object's **title generation** — how
    // many `Created` title effects the journal already has for it. `title.issue/{object}`
    // alone made a re-issue after a write-off an idempotency HIT: the caller got
    // `AlreadyPosted(<the old transaction>)`, which reads as success, and no title was
    // minted. A ruleset handing out the same flat twice would have handed it out once.
    // Two calls in the same generation still resolve to one transaction, which is the
    // property the key exists for.
    let generation = title_generation(ctx, object);
    let actor = PrincipalId::System(issuer);
    let mut plan = EconomyPlan::new(
        crate::record::IdempotencyKey::new(
            SmolStr::new_static("title.issue"),
            SmolStr::new(format!("{}/{generation}", object.0)),
        ),
        actor.clone(),
        EconomicCause::Internal { kind: SmolStr::new_static("title.issue"), id: u64::from(object.0) },
        Vec::new(),
    );
    plan.asset_ops = vec![AssetOp::Title { object, owner: owner.clone(), scope, catalog: None }];
    plan.description = format!("title issued for #{} by {actor:?}", object.0);
    let auth = TitleIssuerAuth { issuer: actor, object, owner, scope };
    post_with(ctx, &plan, PostAuth::Injected(&auth))
}

/// How many title records the journal has already `Created` for `object` (E4c
/// verify fold, E7) — the generation the next issue mints under.
///
/// The **journal** and not the live asset, because that is the record a write-off
/// leaves behind: the asset is `WrittenOff`, the effect that created it is still
/// there, and "how many times has this been titled" is exactly what the key needs.
fn title_generation(ctx: &Ctx<'_>, object: ObjId) -> u64 {
    let Some(store) = ctx.w.module_state::<EconomyStore>() else { return 0 };
    store
        .transactions()
        .flat_map(|t| t.asset_effects.iter())
        .filter(|e| e.object == object && e.kind == crate::record::AssetEffectKind::Created)
        .count() as u64
}

/// Record that this contract's settlement reached the chronicle (E4c verify fold,
/// V2+V3+V5).
///
/// The canon **bridge** calls it — from `mush-server`, after `mush_canon::commit`
/// returned an id — and it is the whole reason publication stopped being
/// best-effort: the marker is durable, so a commit that failed leaves it unset and
/// the sweep re-announces the settlement on its next fire. It is deliberately a
/// narrow, public seam rather than a general "write this contract" API: an observer
/// may record *that it published*, and nothing else.
///
/// Idempotent, and never moves an existing marker: the first line published is the
/// line, and re-pointing it at a second one would be admitting the duplicate this
/// exists to prevent.
pub fn mark_canon_published(ctx: &mut Ctx<'_>, id: crate::ids::ContractId, canon_ref: &str) {
    let Some(mut c) = ctx.w.module_state::<EconomyStore>().and_then(|s| s.snapshot().contract(id).cloned())
    else {
        return;
    };
    if c.canon_published.is_some() {
        return;
    }
    c.canon_published = Some(SmolStr::new(canon_ref));
    c.revision = c.revision.saturating_add(1);
    put_contract(ctx, c);
}

/// The system principal every contract's escrow is made out to (§5).
///
/// It is not a faucet and owns no account. Its whole job is to be a beneficiary
/// **nobody acts for** ([`WorldAuth::acts_for`] returns `false` for every `System`
/// principal, staff included), so a reservation whose purpose is a contract's escrow
/// is capturable only through [`crate::ContractAuth`] — never from a command, never
/// by the payer who placed it, and never by the payee who is owed it.
#[must_use]
pub(crate) fn contract_principal() -> PrincipalId {
    PrincipalId::System(
        mush_org::SystemPrincipalId::new("economy.contract").expect("valid system principal"),
    )
}

/// Install an escalation record into RAM and queue its leaf doc (E4b1, §4.6).
pub(crate) fn put_escalation(ctx: &mut Ctx<'_>, e: crate::escalation::Escalation) {
    {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        store.put_escalation(e.clone());
    }
    persist_escalation(ctx.out, &e);
}

/// Common tail: turn a [`PlanResult`] into a durable-gated outcome or an error.
fn finish(
    ctx: &mut Ctx<'_>,
    outcome: PlanResult,
    providers: &crate::custody::CustodyTable,
    requester: &PrincipalId,
    companions: Vec<crate::contract::EconomicContract>,
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
            let now = ctx.w.now();
            // Prove the exact journal append and every economic post-image
            // representable before custody can move an object. This closes the
            // K0 poison-append seam without changing E3's rule that the world move
            // occurs before the economic state becomes live.
            let commit = prepare_commit(ctx, &prepared, now, companions)?;
            // §4.2 step 2: the world moves BEFORE the store advances, and only from
            // the prepared effects (never the plan's ops — see `AlreadyPosted` above,
            // which performs zero world mutation). **This is the last fallible act
            // of a post** (08c §12): `install_commit` below cannot fail.
            run_custody_effects(ctx, &prepared, providers)?;
            let txid = install_commit(ctx, commit);
            // Invariant 9 / R-R1: the "posted" confirmation must never outrun the
            // durable ack — strict so a dropped write suppresses it entirely.
            ctx.gate_on_durable_ack_strict();
            // E4c §6.1 item 3: the compact events, **post-commit**. Nothing fallible
            // remains, so an observer can never see a state that still rolls back.
            emit_post_events(ctx, txid);
            Ok(PostOutcome::Posted(txid))
        }
    }
}

/// Announce what one committed transaction did (E4c, §6.1 item 3) — **ids and kinds
/// only** (D-E4-13).
///
/// Read back out of the indexed transaction rather than the plan, so an event can
/// only ever describe something the journal has: the reservation ids come from the
/// hold effects, the title ids from the asset effects, and no amount, party, memo or
/// `cause` string is carried anywhere.
fn emit_post_events(ctx: &mut Ctx<'_>, txid: TransactionId) {
    let events: Vec<crate::record::EconomicEvent> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
        if store.observers.is_empty() {
            return;
        }
        let Some(tx) = store.transaction(txid) else { return };
        let mut v = vec![crate::record::EconomicEvent::TransactionPosted { id: txid }];
        for e in &tx.hold_effects {
            v.push(crate::record::EconomicEvent::HoldChanged { id: e.hold.id, kind: e.kind });
        }
        for e in &tx.asset_effects {
            v.push(crate::record::EconomicEvent::AssetTitleChanged {
                object: e.object,
                revision: e.post.revision,
            });
        }
        v
    };
    for ev in &events {
        crate::observer::emit(ctx, ev);
    }
}

/// A fully bounded economy post-image. Building this value is fallible but leaves
/// the world and the economy store untouched; installation only checks that the
/// allocator and ledger tail still match the exact preparation point.
struct PreparedEconomyCommit {
    txid: TransactionId,
    next_transaction: u64,
    journal_seq: u64,
    journal: PreparedLedgerAppend,
    snapshot: EconomySnapshot,
    transaction: EconomicTransaction,
    account_docs: Vec<Account>,
    asset_docs: Vec<EconomicAsset>,
    hold_docs: Vec<Hold>,
    approval_doc: Option<ApprovalRequest>,
    /// Caller-supplied container docs that must be durable with this post (B2).
    contract_docs: Vec<crate::contract::EconomicContract>,
}

fn prepare_commit(
    ctx: &Ctx<'_>,
    prepared: &Prepared,
    now: i64,
    companions: Vec<crate::contract::EconomicContract>,
) -> Result<PreparedEconomyCommit, EconomyError> {
    let store = ctx.w.module_state::<EconomyStore>().expect("economy store");
    let txid = TransactionId(store.peek_next_transaction());
    let next_transaction = store
        .peek_next_transaction()
        .checked_add(1)
        .ok_or(EconomyError::Overflow)?;
    let journal_seq = store.journal.next_seq(JOURNAL_LOG);
    let (mut snapshot, transaction) = apply(store.snapshot(), prepared, txid, journal_seq, now);
    // The companion docs go into the SAME post-image, before it is handed to
    // `install_commit`, so RAM and the batch carry one state (B2).
    for c in &companions {
        snapshot.put_contract(c.clone());
    }
    let journal = store
        .journal
        .prepare_append(
            JOURNAL_LOG,
            serde_json::to_value(&transaction).expect("transaction serializes"),
        )
        .map_err(|error| {
            tracing::error!(%error, "economy journal refused a validated transaction post-image");
            EconomyError::JournalUnavailable
        })?;
    // Every account whose `posted_balance`/`last_journal_seq` moved — AND (E4b1)
    // every account whose `held_amount`/`last_hold_seq` moved, which an effect-only
    // placement does without touching a single money line (D11).
    let touched_accounts: std::collections::BTreeSet<AccountId> = prepared
        .touched_accounts()
        .chain(prepared.hold_effects.iter().map(|e| e.hold.account))
        .collect();
    let account_docs =
        touched_accounts.into_iter().filter_map(|id| snapshot.account(id).cloned()).collect();
    let asset_docs = prepared
        .asset_effects
        .iter()
        .filter_map(|effect| snapshot.asset(effect.object).cloned())
        .collect();
    // Every touched `hold:{id}` projection (§4.2). The post-state is already in the
    // journal entry; this is the read-side doc, healed from the journal at boot.
    let hold_docs = prepared
        .hold_effects
        .iter()
        .filter_map(|effect| snapshot.hold(effect.hold.id).cloned())
        .collect();
    let approval_doc = prepared
        .consumed_approval
        .and_then(|id| snapshot.approval(id).cloned());
    Ok(PreparedEconomyCommit {
        txid,
        next_transaction,
        journal_seq,
        journal,
        snapshot,
        transaction,
        account_docs,
        asset_docs,
        hold_docs,
        approval_doc,
        contract_docs: companions,
    })
}

/// Install a prepared commit. **Infallible** (08c §12, E4b1).
///
/// K's two-phase split left `install_commit` returning `Err` *after*
/// `run_custody_effects` had already moved an object in the world — which
/// contradicts this file's own rule that `post` is a handler's last fallible call,
/// and would leave the world ahead of the journal with nothing to roll it back.
/// E4b1 turns the two post-move checks into **asserted invariants**, keeping the
/// custody move as the last fallible act (`mush-org`'s split is the shape).
///
/// The reachability argument for each, in full, because that is what makes an
/// assertion legitimate rather than optimistic:
///
/// 1. **The allocator cannot move between prepare and install.** `next_transaction`
///    advances in exactly one place — this function — and the world task is
///    single-threaded, so the only way to interleave a second post is for something
///    *between* `prepare_commit` and here to re-enter the economy. The one call in
///    that window is `run_custody_effects`, which drives [`crate::CustodyProvider`]s
///    with a `&mut Ctx`. No shipped provider re-enters: `ContainerProvider` and
///    `mush_sim::InteriorCustodyProvider` move an object and consult the
///    `CarryAttempt` hook chain, whose signature is
///    `carry_attempt(&self, w: &mut World, mover, obj, dest) -> Result<(), RichText>`
///    (`mush_core::module`). That `&mut World` reaches the economy store, but posting
///    needs a `Ctx` — the `Outbox` that carries the journal append, every projection
///    write, and the durable gate — and a hook has none. So a hook can read the
///    economy, and can even mutate the store's own fields, but it **cannot post**:
///    the allocator and the ledger tail have no path to move under it.
/// 2. **The journal tail cannot move either**, for the same reason: `apply_prepared`
///    is the only appender and only this function calls it. E4b1's verify fold (B7)
///    narrowed `EconomyStore::journal` and `::frozen` to `pub(crate)` so that "one
///    mutator" is a property of the type and not a convention — no crate outside this
///    one ever used either field, and both now have read accessors.
///
/// So a fired assertion means a provider (or a future hook) re-entered the posting
/// path — a **programming** error that must be found in test, not a desync silently
/// converted into `JournalUnavailable` after the world had already moved. The
/// regression test in `tests/e4b1.rs` drives exactly such a provider and proves it
/// is the assertion, not a mismatch, that fires.
fn install_commit(ctx: &mut Ctx<'_>, commit: PreparedEconomyCommit) -> TransactionId {
    let PreparedEconomyCommit {
        txid,
        next_transaction,
        journal_seq,
        journal,
        snapshot,
        transaction,
        account_docs,
        asset_docs,
        hold_docs,
        approval_doc,
        contract_docs,
    } = commit;
    let (journal_op, meta) = {
        let store = ctx.w.module_state_mut::<EconomyStore>().expect("economy store");
        assert_eq!(
            store.peek_next_transaction(),
            txid.0,
            "economy: the transaction allocator moved between prepare and install — \
             something re-entered post() after the custody move (see install_commit)"
        );
        let (applied_seq, journal_op) = store.journal.apply_prepared(journal).unwrap_or_else(|error| {
            panic!(
                "economy: the journal tail moved between prepare and install ({error}) — \
                 something re-entered post() after the custody move (see install_commit)"
            )
        });
        assert_eq!(journal_seq, applied_seq, "economy: prepared journal seq is exact");
        store.next_transaction = next_transaction;
        store.snapshot = snapshot;
        store.index_transaction(transaction);
        (journal_op, store.meta_doc())
    };
    persist_journal(ctx.out, journal_op);
    for a in &account_docs {
        persist_account(ctx.out, a);
    }
    for a in &asset_docs {
        persist_asset(ctx.out, a);
    }
    for h in &hold_docs {
        persist_hold(ctx.out, h);
    }
    if let Some(r) = &approval_doc {
        persist_approval(ctx.out, r);
    }
    for c in &contract_docs {
        persist_contract(ctx.out, c);
    }
    persist_meta(ctx.out, meta);
    txid
}
