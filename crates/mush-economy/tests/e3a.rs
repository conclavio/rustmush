//! E3a acceptance — the pure asset / vault / approval engine on hand-built
//! snapshots, exactly the E2a rhythm: every op, every invariant 10–22 refusal, the
//! money+title bundle, plan-hash v2 determinism (including the hard-coded E2 digest
//! pin), and a conservation/shape fuzz.
//!
//! Everything here is world-free. The two live seams are supplied as fakes: an
//! [`AuthEval`] whose every answer is a switch, and a [`CustodyEval`] that stands in
//! for the world containment the pure engine cannot see.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use mush_economy::{
    APPROVAL_TTL_SECS, Account, AccountId, AccountStatus, Amount, Approval, ApprovalRequest,
    ApprovalRequestId, ApprovalRule, ApprovalStatus, AssetEffectKind, AssetLifecycle, AssetOp,
    AssetRevision, AuthEval, CAPABILITIES, CatalogRef, CreditPolicy, CustodyDirection, CustodyEval,
    CustodyOp, CustodyRef, EconomicAsset, EconomicCause, EconomicTransaction, EconomyError,
    EconomyPlan, EconomySnapshot, Encumbrance, IdempotencyKey, InstrumentDef, InstrumentId,
    InstrumentKind, InstrumentStatus, MAX_ASSET_OPS_PER_PLAN, MAX_ENCUMBRANCES_PER_ASSET,
    MAX_REASON_BYTES, NegativePolicy, NoCustody, PlanResult, Prepared, RequestedLine,
    TransactionId, ValidateEnv, Vault, VaultId, VaultStatus, apply, plan_hash, plan_reversal,
    validate,
};
use mush_org::{ApprovalPolicy, OrgId, OrgScopeId, PrincipalId};

// ---- builders -------------------------------------------------------------

const NOW: i64 = 10_000;

fn iid(s: &str) -> InstrumentId {
    InstrumentId::new(s).unwrap()
}
fn obj(n: u32) -> ObjId {
    ObjId(n)
}
fn who(n: u32) -> PrincipalId {
    PrincipalId::Object(ObjId(n))
}
fn org(n: u64) -> PrincipalId {
    PrincipalId::Organization(OrgId(n))
}
fn instrument(id: &str) -> InstrumentDef {
    InstrumentDef {
        id: iid(id),
        ruleset: "test".into(),
        code: id.into(),
        kind: InstrumentKind::Currency,
        scale: 0,
        transferable: true,
        negative_policy: NegativePolicy::SystemOnly,
        status: InstrumentStatus::Active,
        revision: 1,
    }
}
fn account(id: u64, balance: i64) -> Account {
    Account {
        id: AccountId(id),
        owner: who(id as u32),
        instrument: iid("gold"),
        scope: None,
        name: format!("acct{id}").into(),
        status: AccountStatus::Active,
        posted_balance: Amount::from_minor(balance),
        held_amount: Amount::ZERO,
        last_hold_seq: 0,
        credit: CreditPolicy::forbid(),
        last_journal_seq: 0,
        revision: 1,
        approval: None,
    }
}
fn asset(object: u32, owner: PrincipalId) -> EconomicAsset {
    EconomicAsset {
        object: obj(object),
        title_owner: owner,
        scope: None,
        assignment: None,
        maintenance_account: None,
        catalog: None,
        encumbrances: Vec::new(),
        acquisition: None,
        lifecycle: AssetLifecycle::Active,
        last_journal_seq: 0,
        revision: 1,
        frozen: false,
        frozen_reason: None,
    }
}
fn vault(id: u64, backing: u32) -> Vault {
    Vault {
        id: VaultId(id),
        owner: who(1),
        scope: None,
        name: format!("vault{id}").into(),
        backing: CustodyRef::Container(obj(backing)),
        status: VaultStatus::Active,
        revision: 1,
    }
}
/// Two funded accounts (#1 = 1000, #2 = 0) over one instrument.
fn snap() -> EconomySnapshot {
    let mut s = EconomySnapshot::new();
    s.put_instrument(instrument("gold"));
    s.put_account(account(1, 1000));
    s.put_account(account(2, 0));
    s
}
fn line(account: u64, delta: i64) -> RequestedLine {
    RequestedLine { account: AccountId(account), delta: Amount::from_minor(delta), memo: "".into() }
}
fn plan(key: &str, actor: PrincipalId, lines: Vec<RequestedLine>) -> EconomyPlan {
    EconomyPlan::new(
        IdempotencyKey::new("e3a", key),
        actor,
        EconomicCause::Staff { reason: "test".into() },
        lines,
    )
}
/// A plan carrying only asset ops (the effect-only shape invariant 10 admits).
fn asset_plan(key: &str, actor: PrincipalId, ops: Vec<AssetOp>) -> EconomyPlan {
    let mut p = plan(key, actor, Vec::new());
    p.asset_ops = ops;
    p
}
fn lien(creditor: u32, amount: i64) -> Encumbrance {
    Encumbrance::Lien {
        creditor: who(creditor),
        amount: Amount::from_minor(amount),
        instrument: iid("gold"),
    }
}
fn disputed(claimant: u32, reason: &str) -> Encumbrance {
    Encumbrance::Disputed { claimant: who(claimant), reason: reason.into() }
}

// ---- the seams as fakes ---------------------------------------------------

/// Every answer of the authorization seam as a switch, so a test can deny exactly
/// one axis and prove the engine actually asks about it.
#[derive(Clone)]
struct Auth {
    money: bool,
    asset: bool,
    receipt: bool,
    vault: bool,
    maintenance: bool,
    /// Which objects currently qualify as approvers (re-read on every call, which is
    /// how the "an approver who no longer qualifies" test works).
    approvers: BTreeSet<ObjId>,
}
impl Auth {
    fn yes() -> Auth {
        Auth {
            money: true,
            asset: true,
            receipt: true,
            vault: true,
            maintenance: true,
            approvers: BTreeSet::new(),
        }
    }
    fn approvers(mut self, ids: &[u32]) -> Auth {
        self.approvers = ids.iter().map(|n| obj(*n)).collect();
        self
    }
}
impl AuthEval for Auth {
    fn authorize(&self, _: &PrincipalId, _: &Account, _: Amount) -> bool {
        self.money
    }
    fn authorize_asset(&self, _: &PrincipalId, _: Option<&EconomicAsset>, _: &AssetOp) -> bool {
        self.asset
    }
    fn authorize_title_receipt(&self, _: &PrincipalId, _: &PrincipalId, _: Option<OrgScopeId>) -> bool {
        self.receipt
    }
    fn authorize_vault(&self, _: &PrincipalId, _: &Vault, _: &CustodyOp) -> bool {
        self.vault
    }
    fn authorize_maintenance(&self, _: &PrincipalId, _: &Account) -> bool {
        self.maintenance
    }
    fn authorize_approver(&self, approver: ObjId, _: &Account, _: &ApprovalPolicy) -> bool {
        self.approvers.contains(&approver)
    }
}

/// The world facts the pure engine cannot see, as data.
#[derive(Clone, Default)]
struct Custody {
    valid: bool,
    standing: bool,
    destination: bool,
    held: BTreeSet<ObjId>,
}
impl Custody {
    fn open() -> Custody {
        Custody { valid: true, standing: true, destination: true, held: BTreeSet::new() }
    }
    fn holding(mut self, ids: &[u32]) -> Custody {
        self.held = ids.iter().map(|n| obj(*n)).collect();
        self
    }
}
impl CustodyEval for Custody {
    fn backing_valid(&self, _: &Vault) -> bool {
        self.valid
    }
    fn holds(&self, _: &Vault, o: ObjId) -> bool {
        self.held.contains(&o)
    }
    fn may_deposit(&self, _: ObjId, _: &PrincipalId) -> bool {
        self.standing
    }
    fn destination_ok(&self, _: &PrincipalId, _: ObjId) -> bool {
        self.destination
    }
}

// ---- drivers --------------------------------------------------------------

fn env<'a>(auth: &'a dyn AuthEval, custody: &'a dyn CustodyEval) -> ValidateEnv<'a> {
    ValidateEnv { auth, custody, facts: &mush_economy::NoFacts, now: NOW, ic_now: NOW, next_hold_id: 1 }
}
fn prepare(s: &EconomySnapshot, p: &EconomyPlan, auth: &Auth, custody: &Custody) -> Prepared {
    match validate(p, s, &env(auth, custody)) {
        PlanResult::Post(prepared) => *prepared,
        other => panic!("expected Post, got {other:?}"),
    }
}
/// Validate + apply with the fully-permissive seams (the common happy path).
fn post(s: &EconomySnapshot, p: &EconomyPlan, txid: u64) -> (EconomySnapshot, EconomicTransaction) {
    post_with(s, p, txid, &Auth::yes(), &Custody::open())
}
fn post_with(
    s: &EconomySnapshot,
    p: &EconomyPlan,
    txid: u64,
    auth: &Auth,
    custody: &Custody,
) -> (EconomySnapshot, EconomicTransaction) {
    let prepared = prepare(s, p, auth, custody);
    apply(s, &prepared, TransactionId(txid), txid + 100, NOW)
}
fn refuse_with(s: &EconomySnapshot, p: &EconomyPlan, auth: &Auth, custody: &Custody) -> EconomyError {
    match validate(p, s, &env(auth, custody)) {
        PlanResult::Refused(e) => e,
        other => panic!("expected Refused, got {other:?}"),
    }
}
fn refuse(s: &EconomySnapshot, p: &EconomyPlan) -> EconomyError {
    refuse_with(s, p, &Auth::yes(), &Custody::open())
}
fn needs_approval(s: &EconomySnapshot, p: &EconomyPlan, auth: &Auth) -> Vec<AccountId> {
    match validate(p, s, &env(auth, &NoCustody)) {
        PlanResult::NeedsApproval(need) => {
            assert_eq!(need.plan_hash, plan_hash(p), "the need names the plan's own hash");
            need.accounts
        }
        other => panic!("expected NeedsApproval, got {other:?}"),
    }
}
fn bal(s: &EconomySnapshot, id: u64) -> i64 {
    s.account(AccountId(id)).unwrap().posted_balance.minor()
}

// ---- invariant 11 / 22: creating and moving title -------------------------

#[test]
fn title_creates_an_asset_effect_only_with_no_acquisition() {
    // Invariant 10: a transaction with ZERO money lines but one effect is legal.
    let s = snap();
    let p = asset_plan("title", who(1), vec![AssetOp::Title {
        object: obj(50),
        owner: who(2),
        scope: None,
        catalog: Some(CatalogRef { ruleset: "sr".into(), code: "ares.predator".into() }),
    }]);
    let (s2, tx) = post(&s, &p, 1);
    let a = s2.asset(obj(50)).expect("asset created");
    assert_eq!(a.title_owner, who(2));
    assert_eq!(a.revision, 1, "a fresh title starts at revision 1");
    assert_eq!(a.last_journal_seq, 101, "apply stamps the journal sequence");
    assert_eq!(a.lifecycle, AssetLifecycle::Active);
    // Invariant 22: no money moved, so this was not an acquisition.
    assert_eq!(a.acquisition, None);
    assert!(tx.lines.is_empty(), "an effect-only entry carries no money lines");
    assert_eq!(tx.asset_effects.len(), 1);
    assert_eq!(tx.asset_effects[0].kind, AssetEffectKind::Created);
    assert_eq!(tx.asset_effects[0].post.catalog.as_ref().unwrap().code, "ares.predator");
    // The owner index sees it.
    assert_eq!(s2.assets_of(&who(2)).count(), 1);
}

#[test]
fn title_refuses_an_object_that_already_has_one() {
    let mut s = snap();
    s.put_asset(asset(50, who(2)));
    let p = asset_plan("dup", who(1), vec![AssetOp::Title {
        object: obj(50),
        owner: who(3),
        scope: None,
        catalog: None,
    }]);
    assert_eq!(refuse(&s, &p), EconomyError::AssetExists(obj(50)));
}

#[test]
fn every_asset_op_asks_the_seam_and_a_denial_refuses() {
    // Invariants 19/20 live in the seam; the pure engine's job is to ASK, always.
    let mut s = snap();
    s.put_asset(asset(50, who(2)));
    let mut deny = Auth::yes();
    deny.asset = false;
    for op in [
        AssetOp::Title { object: obj(51), owner: who(2), scope: None, catalog: None },
        AssetOp::TransferTitle { object: obj(50), to: who(3), to_scope: None },
        AssetOp::Assign { object: obj(50), to: who(3), purpose: "loan".into(), valid_until: None },
        AssetOp::Unassign { object: obj(50) },
        AssetOp::Encumber { object: obj(50), encumbrance: lien(9, 100) },
        AssetOp::SetMaintenance { object: obj(50), account: None },
        AssetOp::WriteOff { object: obj(50), reason: "scrapped".into() },
    ] {
        let p = asset_plan("deny", who(1), vec![op.clone()]);
        assert_eq!(refuse_with(&s, &p, &deny, &Custody::open()), EconomyError::Unauthorized, "{op:?}");
    }
}

#[test]
fn money_and_title_move_in_one_transaction_and_stamp_acquisition() {
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    let mut p = plan("sale", who(1), vec![line(2, -250), line(1, 250)]);
    p.asset_ops = vec![AssetOp::TransferTitle { object: obj(50), to: who(2), to_scope: None }];
    let mut s2 = s.clone();
    s2.put_account(account(2, 400));
    let (s3, tx) = post(&s2, &p, 7);
    assert_eq!(bal(&s3, 1), 1250);
    assert_eq!(bal(&s3, 2), 150);
    let a = s3.asset(obj(50)).unwrap();
    assert_eq!(a.title_owner, who(2), "title moved in the same bundle");
    assert_eq!(a.revision, 2, "one effect bumps the revision once");
    // Invariant 22: money moved, so provenance is stamped with THIS transaction.
    assert_eq!(a.acquisition.unwrap().transaction, TransactionId(7));
    assert_eq!(a.acquisition.unwrap().at, NOW);
    assert_eq!(tx.asset_effects[0].kind, AssetEffectKind::TitleTransferred);
    assert_eq!(tx.asset_effects[0].post.acquisition.unwrap().transaction, TransactionId(7));
}

#[test]
fn transfer_clears_the_assignment_and_the_maintenance_payer() {
    // Invariant 13 / A8: both are relationships with the OLD owner, and title
    // carries liabilities the new owner never agreed to.
    let mut s = snap();
    let mut a = asset(50, who(1));
    a.assignment = Some(mush_economy::AssetAssignment {
        assigned_to: who(3),
        purpose: "field use".into(),
        valid_until: None,
    });
    a.maintenance_account = Some(AccountId(1));
    s.put_asset(a);
    let p = asset_plan("xfer", who(1), vec![AssetOp::TransferTitle {
        object: obj(50),
        to: who(2),
        to_scope: None,
    }]);
    let (s2, tx) = post(&s, &p, 1);
    let a = s2.asset(obj(50)).unwrap();
    assert_eq!(a.assignment, None);
    assert_eq!(a.maintenance_account, None);
    assert_eq!(tx.asset_effects[0].post.assignment, None, "the effect records the clearing");
}

#[test]
fn transfer_and_write_off_refuse_an_encumbered_asset() {
    // Invariant 12.
    let mut s = snap();
    let mut a = asset(50, who(1));
    a.encumbrances = vec![lien(9, 500)];
    s.put_asset(a);
    let xfer = asset_plan("x", who(1), vec![AssetOp::TransferTitle {
        object: obj(50),
        to: who(2),
        to_scope: None,
    }]);
    assert_eq!(refuse(&s, &xfer), EconomyError::Encumbered(obj(50)));
    let off = asset_plan("o", who(1), vec![AssetOp::WriteOff {
        object: obj(50),
        reason: "lost".into(),
    }]);
    assert_eq!(refuse(&s, &off), EconomyError::Encumbered(obj(50)));
}

#[test]
fn releases_apply_first_so_release_plus_transfer_is_one_legal_plan() {
    // Invariant 11/12 together: the ONLY way to move an encumbered title is to clear
    // every claim in the same plan, and releases sort ahead of the title change.
    let mut s = snap();
    let mut a = asset(50, who(1));
    a.encumbrances = vec![lien(9, 500), disputed(8, "ownership")];
    s.put_asset(a);
    let p = asset_plan("rel", who(1), vec![
        AssetOp::TransferTitle { object: obj(50), to: who(2), to_scope: None },
        AssetOp::Release { object: obj(50), encumbrance: disputed(8, "ownership") },
        AssetOp::Release { object: obj(50), encumbrance: lien(9, 500) },
    ]);
    let (s2, tx) = post(&s, &p, 1);
    let a = s2.asset(obj(50)).unwrap();
    assert!(a.encumbrances.is_empty());
    assert_eq!(a.title_owner, who(2));
    assert_eq!(a.revision, 4, "three effects, three revision bumps");
    let kinds: Vec<_> = tx.asset_effects.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, vec![
        AssetEffectKind::Released,
        AssetEffectKind::Released,
        AssetEffectKind::TitleTransferred
    ]);
}

#[test]
fn release_refuses_a_claim_the_asset_does_not_carry() {
    let mut s = snap();
    let mut a = asset(50, who(1));
    a.encumbrances = vec![lien(9, 500)];
    s.put_asset(a);
    // Exact match: a lien for a different amount is a different lien.
    let p = asset_plan("r", who(1), vec![AssetOp::Release {
        object: obj(50),
        encumbrance: lien(9, 499),
    }]);
    assert_eq!(refuse(&s, &p), EconomyError::NoSuchEncumbrance(obj(50)));
}

#[test]
fn two_title_changing_ops_on_one_object_refuse() {
    // Invariant 11: Title / TransferTitle / WriteOff are mutually exclusive.
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    let p = asset_plan("two", who(1), vec![
        AssetOp::TransferTitle { object: obj(50), to: who(2), to_scope: None },
        AssetOp::WriteOff { object: obj(50), reason: "gone".into() },
    ]);
    assert_eq!(refuse(&s, &p), EconomyError::DuplicateAssetOp(obj(50)));
    // ...and so do two assignment-changing ops.
    let p = asset_plan("two2", who(1), vec![
        AssetOp::Assign { object: obj(50), to: who(3), purpose: "a".into(), valid_until: None },
        AssetOp::Unassign { object: obj(50) },
    ]);
    assert_eq!(refuse(&s, &p), EconomyError::DuplicateAssetOp(obj(50)));
}

#[test]
fn an_exactly_duplicated_op_refuses() {
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    let p = asset_plan("d", who(1), vec![
        AssetOp::Encumber { object: obj(50), encumbrance: lien(9, 5) },
        AssetOp::Encumber { object: obj(50), encumbrance: lien(9, 5) },
    ]);
    assert_eq!(refuse(&s, &p), EconomyError::DuplicateAssetOp(obj(50)));
}

#[test]
fn assign_refuses_the_title_owner_and_records_the_assignee_otherwise() {
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    let bad = asset_plan("self", who(1), vec![AssetOp::Assign {
        object: obj(50),
        to: who(1),
        purpose: "mine".into(),
        valid_until: None,
    }]);
    assert_eq!(refuse(&s, &bad), EconomyError::AssignmentToOwner(obj(50)));
    let good = asset_plan("ok", who(1), vec![AssetOp::Assign {
        object: obj(50),
        to: who(3),
        purpose: "field use".into(),
        valid_until: Some(NOW + 60),
    }]);
    let (s2, _) = post(&s, &good, 1);
    let assignment = s2.asset(obj(50)).unwrap().assignment.clone().unwrap();
    assert_eq!(assignment.assigned_to, who(3));
    assert_eq!(assignment.valid_until, Some(NOW + 60));
    // Unassign clears it again.
    let p = asset_plan("un", who(1), vec![AssetOp::Unassign { object: obj(50) }]);
    let (s3, tx) = post(&s2, &p, 2);
    assert_eq!(s3.asset(obj(50)).unwrap().assignment, None);
    assert_eq!(tx.asset_effects[0].kind, AssetEffectKind::Unassigned);
}

#[test]
fn set_maintenance_needs_a_related_account_and_debit_authority() {
    // Invariant 14 (design-fit A.4): the payer must belong to the asset's owner or
    // share its compartment, AND the actor must be able to DEBIT it — crediting is
    // free, so a zero-delta probe through `authorize` would prove nothing.
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    // Account #2 is owned by object #2, the asset by object #1: unrelated.
    let unrelated = asset_plan("m1", who(1), vec![AssetOp::SetMaintenance {
        object: obj(50),
        account: Some(AccountId(2)),
    }]);
    assert_eq!(refuse(&s, &unrelated), EconomyError::ScopeOwnerMismatch);
    // Account #1 IS owned by object #1 — but a seam that denies the debit refuses.
    let ok = asset_plan("m2", who(1), vec![AssetOp::SetMaintenance {
        object: obj(50),
        account: Some(AccountId(1)),
    }]);
    let mut deny = Auth::yes();
    deny.maintenance = false;
    assert_eq!(refuse_with(&s, &ok, &deny, &Custody::open()), EconomyError::Unauthorized);
    let (s2, _) = post(&s, &ok, 1);
    assert_eq!(s2.asset(obj(50)).unwrap().maintenance_account, Some(AccountId(1)));
    // A shared org compartment is the other admissible relation.
    let mut s3 = snap();
    let mut a = asset(50, org(4));
    a.scope = Some(OrgScopeId(9));
    s3.put_asset(a);
    let mut acct = account(3, 0);
    acct.owner = org(4);
    acct.scope = Some(OrgScopeId(9));
    s3.put_account(acct);
    let scoped = asset_plan("m3", who(1), vec![AssetOp::SetMaintenance {
        object: obj(50),
        account: Some(AccountId(3)),
    }]);
    let (s4, _) = post(&s3, &scoped, 1);
    assert_eq!(s4.asset(obj(50)).unwrap().maintenance_account, Some(AccountId(3)));
}

#[test]
fn write_off_is_terminal_and_later_ops_refuse() {
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    let p = asset_plan("off", who(1), vec![AssetOp::WriteOff {
        object: obj(50),
        reason: "destroyed in play".into(),
    }]);
    let (s2, tx) = post(&s, &p, 1);
    assert_eq!(s2.asset(obj(50)).unwrap().lifecycle, AssetLifecycle::WrittenOff);
    assert_eq!(tx.asset_effects[0].kind, AssetEffectKind::WrittenOff);
    // Invariant 11: a written-off asset takes no further op.
    let after = asset_plan("after", who(1), vec![AssetOp::Unassign { object: obj(50) }]);
    assert_eq!(refuse(&s2, &after), EconomyError::AssetNotActive(obj(50)));
}

#[test]
fn a_frozen_asset_and_a_missing_asset_both_refuse() {
    let mut s = snap();
    let mut a = asset(50, who(1));
    a.frozen = true;
    a.frozen_reason = Some("recovery".into());
    s.put_asset(a);
    let p = asset_plan("f", who(1), vec![AssetOp::Unassign { object: obj(50) }]);
    assert_eq!(refuse(&s, &p), EconomyError::AssetFrozen(obj(50)));
    let ghost = asset_plan("g", who(1), vec![AssetOp::Unassign { object: obj(99) }]);
    assert_eq!(refuse(&s, &ghost), EconomyError::NoSuchAsset(obj(99)));
}

#[test]
fn scope_and_owner_must_agree_on_both_ends_of_a_title() {
    // Invariant 18 / A4: an attacker-chosen `to_scope` must not smuggle an asset into
    // an org compartment it does not belong to.
    let s = snap();
    let bad_create = asset_plan("s1", who(1), vec![AssetOp::Title {
        object: obj(50),
        owner: who(2),
        scope: Some(OrgScopeId(3)),
        catalog: None,
    }]);
    assert_eq!(refuse(&s, &bad_create), EconomyError::ScopeOwnerMismatch);
    let mut s2 = snap();
    s2.put_asset(asset(50, who(1)));
    let bad_xfer = asset_plan("s2", who(1), vec![AssetOp::TransferTitle {
        object: obj(50),
        to: who(2),
        to_scope: Some(OrgScopeId(3)),
    }]);
    assert_eq!(refuse(&s2, &bad_xfer), EconomyError::ScopeOwnerMismatch);
    // An organization principal WITH a scope is coherent.
    let good = asset_plan("s3", who(1), vec![AssetOp::TransferTitle {
        object: obj(50),
        to: org(7),
        to_scope: Some(OrgScopeId(3)),
    }]);
    let (s3, _) = post(&s2, &good, 1);
    assert_eq!(s3.asset(obj(50)).unwrap().scope, Some(OrgScopeId(3)));
}

#[test]
fn a_transfer_needs_authority_on_the_destination_too() {
    // D-E3-12: the source half is `authorize_asset`, the destination half is its own
    // question, because title carries liabilities a money credit does not.
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    let mut deny = Auth::yes();
    deny.receipt = false;
    let p = asset_plan("recv", who(1), vec![AssetOp::TransferTitle {
        object: obj(50),
        to: who(2),
        to_scope: None,
    }]);
    assert_eq!(refuse_with(&s, &p, &deny, &Custody::open()), EconomyError::Unauthorized);
}

#[test]
fn an_effect_carries_the_full_post_state_with_the_latch_zeroed() {
    // D-E3-1: replay must be able to rebuild the asset from the effect ALONE, and the
    // `frozen` latch is doc authority that never rides the journal.
    let mut s = snap();
    let mut a = asset(50, who(1));
    a.catalog = Some(CatalogRef { ruleset: "sr".into(), code: "van".into() });
    a.encumbrances = vec![disputed(8, "claim")];
    s.put_asset(a);
    let p = asset_plan("full", who(1), vec![AssetOp::Assign {
        object: obj(50),
        to: who(3),
        purpose: "haul".into(),
        valid_until: None,
    }]);
    let (_s2, tx) = post(&s, &p, 3);
    let post_state = &tx.asset_effects[0].post;
    assert_eq!(post_state.object, obj(50));
    assert_eq!(post_state.title_owner, who(1));
    assert_eq!(post_state.catalog.as_ref().unwrap().code, "van");
    assert_eq!(post_state.encumbrances.len(), 1, "unrelated state rides along in full");
    assert_eq!(post_state.revision, 2);
    assert_eq!(post_state.last_journal_seq, 103);
    assert!(!post_state.frozen, "the latch is never inside an effect");
    assert_eq!(post_state.frozen_reason, None);
}

#[test]
fn a_stale_expected_asset_revision_is_refused() {
    // Invariant 16.
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    let mut p = asset_plan("stale", who(1), vec![AssetOp::Unassign { object: obj(50) }]);
    p.expected_asset_revisions = vec![AssetRevision { object: obj(50), revision: 99 }];
    assert_eq!(refuse(&s, &p), EconomyError::StaleAsset {
        object: obj(50),
        expected: 99,
        found: Some(1),
    });
    // The matching revision passes.
    p.expected_asset_revisions = vec![AssetRevision { object: obj(50), revision: 1 }];
    let _ = post(&s, &p, 1);
}

#[test]
fn a_transaction_needs_two_money_lines_or_one_effect() {
    // Invariant 10, both halves.
    let s = snap();
    assert_eq!(refuse(&s, &plan("empty", who(1), Vec::new())), EconomyError::TooFewLines);
    // One money line + an effect is still refused: when money lines exist at all,
    // invariants 1-4 hold unchanged.
    let mut p = plan("one", who(1), vec![line(1, -10)]);
    p.asset_ops = vec![AssetOp::Title { object: obj(50), owner: who(2), scope: None, catalog: None }];
    assert_eq!(refuse(&s, &p), EconomyError::TooFewLines);
}

// ---- invariant 15: custody ------------------------------------------------

#[test]
fn deposit_and_withdraw_build_audit_effects_without_touching_title() {
    let mut s = snap();
    s.put_vault(vault(1, 80));
    s.put_asset(asset(50, who(1)));
    let mut p = plan("dep", who(1), Vec::new());
    p.custody_ops = vec![CustodyOp::Deposit { object: obj(50), vault: VaultId(1) }];
    let (s2, tx) = post_with(&s, &p, 1, &Auth::yes(), &Custody::open());
    assert_eq!(tx.custody_effects.len(), 1);
    assert_eq!(tx.custody_effects[0].direction, CustodyDirection::Deposit);
    assert_eq!(tx.custody_effects[0].to, obj(80), "the effect names the backing object");
    assert_eq!(tx.custody_effects[0].from, None);
    // D5: custody moved, title did not.
    assert_eq!(s2.asset(obj(50)).unwrap().title_owner, who(1));
    assert_eq!(s2.asset(obj(50)).unwrap().revision, 1, "no asset effect, no revision bump");

    let mut w = plan("wd", who(1), Vec::new());
    w.custody_ops = vec![CustodyOp::Withdraw { object: obj(50), vault: VaultId(1), to: obj(4) }];
    let (_s3, tx) = post_with(&s2, &w, 2, &Auth::yes(), &Custody::open().holding(&[50]));
    assert_eq!(tx.custody_effects[0].direction, CustodyDirection::Withdraw);
    assert_eq!(tx.custody_effects[0].from, Some(obj(80)));
    assert_eq!(tx.custody_effects[0].to, obj(4));
}

#[test]
fn custody_refuses_without_standing_holding_or_a_valid_backing() {
    let mut s = snap();
    s.put_vault(vault(1, 80));
    let mut dep = plan("d", who(1), Vec::new());
    dep.custody_ops = vec![CustodyOp::Deposit { object: obj(50), vault: VaultId(1) }];
    // No custody standing over the object.
    let mut no_standing = Custody::open();
    no_standing.standing = false;
    assert_eq!(
        refuse_with(&s, &dep, &Auth::yes(), &no_standing),
        EconomyError::NoCustodyStanding(obj(50))
    );
    // An invalid backing (and `NoCustody`, the E3a default) refuses fail-closed.
    let mut bad_backing = Custody::open();
    bad_backing.valid = false;
    assert_eq!(
        refuse_with(&s, &dep, &Auth::yes(), &bad_backing),
        EconomyError::BackingInvalid(VaultId(1))
    );
    match validate(&dep, &s, &env(&Auth::yes(), &NoCustody)) {
        PlanResult::Refused(EconomyError::BackingInvalid(_)) => {}
        other => panic!("NoCustody must refuse every custody op, got {other:?}"),
    }
    // Withdraw of something the vault does not hold.
    let mut wd = plan("w", who(1), Vec::new());
    wd.custody_ops = vec![CustodyOp::Withdraw { object: obj(50), vault: VaultId(1), to: obj(4) }];
    assert_eq!(
        refuse_with(&s, &wd, &Auth::yes(), &Custody::open()),
        EconomyError::NotInVault(obj(50))
    );
    // The vault's own policy is asked, live.
    let mut deny = Auth::yes();
    deny.vault = false;
    assert_eq!(refuse_with(&s, &dep, &deny, &Custody::open()), EconomyError::Unauthorized);
    // A non-active vault takes nothing.
    let mut frozen = vault(1, 80);
    frozen.status = VaultStatus::Frozen;
    s.put_vault(frozen);
    assert_eq!(
        refuse_with(&s, &dep, &Auth::yes(), &Custody::open()),
        EconomyError::VaultNotActive(VaultId(1))
    );
    // A missing vault.
    let mut ghost = plan("g", who(1), Vec::new());
    ghost.custody_ops = vec![CustodyOp::Deposit { object: obj(50), vault: VaultId(9) }];
    assert_eq!(refuse(&s, &ghost), EconomyError::NoSuchVault(VaultId(9)));
}

#[test]
fn a_withdrawal_may_not_land_inside_another_vault() {
    // A2: entering a vault is a Deposit — with its own standing check — never a
    // withdrawal's side effect (the container-laundering path).
    let mut s = snap();
    s.put_vault(vault(1, 80));
    s.put_vault(vault(2, 81));
    let mut p = plan("launder", who(1), Vec::new());
    p.custody_ops = vec![CustodyOp::Withdraw { object: obj(50), vault: VaultId(1), to: obj(81) }];
    assert_eq!(
        refuse_with(&s, &p, &Auth::yes(), &Custody::open().holding(&[50])),
        EconomyError::BadDestination(obj(81))
    );
    // A destination the world seam rejects (dead / GOING / unreachable) refuses too.
    let mut p2 = plan("dest", who(1), Vec::new());
    p2.custody_ops = vec![CustodyOp::Withdraw { object: obj(50), vault: VaultId(1), to: obj(4) }];
    let mut bad = Custody::open().holding(&[50]);
    bad.destination = false;
    assert_eq!(refuse_with(&s, &p2, &Auth::yes(), &bad), EconomyError::BadDestination(obj(4)));
}

#[test]
fn only_one_custody_op_per_object_per_plan() {
    let mut s = snap();
    s.put_vault(vault(1, 80));
    s.put_vault(vault(2, 81));
    let mut p = plan("two", who(1), Vec::new());
    p.custody_ops = vec![
        CustodyOp::Deposit { object: obj(50), vault: VaultId(1) },
        CustodyOp::Deposit { object: obj(50), vault: VaultId(2) },
    ];
    assert_eq!(
        refuse_with(&s, &p, &Auth::yes(), &Custody::open()),
        EconomyError::DuplicateCustodyOp(obj(50))
    );
}

#[test]
fn a_registered_backing_has_no_world_object_and_is_refused() {
    let mut s = snap();
    let mut v = vault(1, 80);
    v.backing = CustodyRef::Registered { provider: "sim".into(), id: 7 };
    s.put_vault(v);
    let mut p = plan("reg", who(1), Vec::new());
    p.custody_ops = vec![CustodyOp::Deposit { object: obj(50), vault: VaultId(1) }];
    assert_eq!(
        refuse_with(&s, &p, &Auth::yes(), &Custody::open()),
        EconomyError::BackingInvalid(VaultId(1))
    );
}

// ---- invariant 17: approval ------------------------------------------------

fn rule(threshold: i64, approvals: u8) -> ApprovalRule {
    ApprovalRule {
        threshold: Amount::from_minor(threshold),
        policy: ApprovalPolicy::Threshold { approvals, from_role: None },
    }
}
/// Snapshot with account #1 carrying an approval rule.
fn snap_with_rule(threshold: i64, approvals: u8) -> EconomySnapshot {
    let mut s = snap();
    let mut a = account(1, 1000);
    a.approval = Some(rule(threshold, approvals));
    s.put_account(a);
    s
}
fn request(id: u64, hash: [u8; 32], approvers: &[u32], requested_by: PrincipalId) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalRequestId(id),
        plan_hash: hash,
        accounts: vec![AccountId(1)],
        requested_by,
        approvals: approvers.iter().map(|n| Approval { approver: obj(*n), at: NOW - 10 }).collect(),
        decisions: Vec::new(),
        created_at: NOW - 10,
        expires_at: NOW - 10 + APPROVAL_TTL_SECS,
        status: ApprovalStatus::Pending,
        revision: 1,
    }
}

#[test]
fn an_approval_rule_fires_only_above_its_threshold() {
    let s = snap_with_rule(100, 2);
    let auth = Auth::yes();
    // At the threshold: no escalation (strictly greater).
    let at = plan("at", who(1), vec![line(1, -100), line(2, 100)]);
    assert!(validate(&at, &s, &env(&auth, &NoCustody)).is_post());
    // Below: none either.
    let below = plan("below", who(1), vec![line(1, -99), line(2, 99)]);
    assert!(validate(&below, &s, &env(&auth, &NoCustody)).is_post());
    // Above: escalation, naming the account whose rule fired.
    let above = plan("above", who(1), vec![line(1, -101), line(2, 101)]);
    assert_eq!(needs_approval(&s, &above, &auth), vec![AccountId(1)]);
    // A NET CREDIT on the ruled account never escalates, however large.
    let credit = plan("credit", who(2), vec![line(1, 5000), line(2, -5000)]);
    let mut s2 = s.clone();
    s2.put_account(Account { credit: CreditPolicy::unbounded(), ..account(2, 0) });
    assert!(validate(&credit, &s2, &env(&auth, &NoCustody)).is_post());
}

#[test]
fn a_satisfied_request_lets_the_plan_post_and_is_consumed_once() {
    let mut s = snap_with_rule(100, 2);
    let p = plan("big", who(1), vec![line(1, -500), line(2, 500)]);
    s.put_approval(request(1, plan_hash(&p), &[7, 8], who(9)));
    let auth = Auth::yes().approvers(&[7, 8]);
    let prepared = prepare(&s, &p, &auth, &Custody::open());
    assert_eq!(prepared.consumed_approval, Some(ApprovalRequestId(1)));
    let (s2, tx) = apply(&s, &prepared, TransactionId(4), 4, NOW);
    assert_eq!(tx.consumed_approval, Some(ApprovalRequestId(1)));
    assert_eq!(bal(&s2, 1), 500);
    // At most once: the request is spent and no longer live for its hash.
    assert_eq!(
        s2.approval(ApprovalRequestId(1)).unwrap().status,
        ApprovalStatus::Consumed { transaction: TransactionId(4) }
    );
    assert!(s2.live_approval_for_hash(&tx.plan_hash).is_none());
}

#[test]
fn approvers_are_re_qualified_live_and_the_requester_never_counts() {
    let mut s = snap_with_rule(100, 2);
    let p = plan("live", who(1), vec![line(1, -500), line(2, 500)]);
    s.put_approval(request(1, plan_hash(&p), &[7, 8], who(9)));
    // A5: one approver has since lost the capability — the count drops to 1.
    let lost = Auth::yes().approvers(&[7]);
    assert_eq!(needs_approval(&s, &p, &lost), vec![AccountId(1)]);
    // Four-eyes: the requester's own approval does not count.
    let mut s2 = snap_with_rule(100, 2);
    s2.put_approval(request(2, plan_hash(&p), &[7, 9], who(9)));
    let both = Auth::yes().approvers(&[7, 9]);
    assert_eq!(needs_approval(&s2, &p, &both), vec![AccountId(1)]);
    // The same two approvals, from two people who are not the requester, satisfy it.
    let mut s3 = snap_with_rule(100, 2);
    s3.put_approval(request(3, plan_hash(&p), &[7, 8], who(9)));
    assert!(validate(&p, &s3, &env(&Auth::yes().approvers(&[7, 8]), &NoCustody)).is_post());
}

#[test]
fn mutating_an_approved_plan_needs_approval_again() {
    // Scenario 11, pure half: the approval binds the HASH, so any economic change
    // (here, the amount) orphans it.
    let mut s = snap_with_rule(100, 2);
    let approved = plan("m", who(1), vec![line(1, -500), line(2, 500)]);
    s.put_approval(request(1, plan_hash(&approved), &[7, 8], who(9)));
    let auth = Auth::yes().approvers(&[7, 8]);
    assert!(validate(&approved, &s, &env(&auth, &NoCustody)).is_post());
    let mutated = plan("m", who(1), vec![line(1, -600), line(2, 600)]);
    assert_ne!(plan_hash(&approved), plan_hash(&mutated));
    assert_eq!(needs_approval(&s, &mutated, &auth), vec![AccountId(1)]);
}

#[test]
fn approvals_authorize_an_intent_not_a_state() {
    // `expected_*_revisions` are OUTSIDE the hash, so a request approved at revision
    // N still authorizes the same plan at N+k.
    let mut s = snap_with_rule(100, 2);
    let mut p = plan("intent", who(1), vec![line(1, -500), line(2, 500)]);
    let hash = plan_hash(&p);
    p.expected_revisions = vec![mush_economy::AccountRevision { account: AccountId(1), revision: 1 }];
    assert_eq!(plan_hash(&p), hash, "concurrency metadata is not part of the identity");
    s.put_approval(request(1, hash, &[7, 8], who(9)));
    assert!(validate(&p, &s, &env(&Auth::yes().approvers(&[7, 8]), &NoCustody)).is_post());
}

#[test]
fn an_expired_or_missing_request_needs_approval_rather_than_refusing() {
    let mut s = snap_with_rule(100, 1);
    let p = plan("exp", who(1), vec![line(1, -500), line(2, 500)]);
    // Missing entirely.
    assert_eq!(needs_approval(&s, &p, &Auth::yes().approvers(&[7])), vec![AccountId(1)]);
    // Present, satisfied, but aged out.
    let mut req = request(1, plan_hash(&p), &[7], who(9));
    req.created_at = NOW - APPROVAL_TTL_SECS - 1;
    req.expires_at = NOW - 1;
    s.put_approval(req);
    assert_eq!(needs_approval(&s, &p, &Auth::yes().approvers(&[7])), vec![AccountId(1)]);
}

#[test]
fn a_request_that_does_not_cover_the_account_does_not_satisfy_it() {
    let mut s = snap_with_rule(100, 1);
    let p = plan("cover", who(1), vec![line(1, -500), line(2, 500)]);
    let mut req = request(1, plan_hash(&p), &[7], who(9));
    req.accounts = vec![AccountId(2)];
    s.put_approval(req);
    assert_eq!(needs_approval(&s, &p, &Auth::yes().approvers(&[7])), vec![AccountId(1)]);
}

#[test]
fn a_consumed_or_cancelled_request_is_not_live() {
    let mut s = snap_with_rule(100, 1);
    let p = plan("dead", who(1), vec![line(1, -500), line(2, 500)]);
    let mut req = request(1, plan_hash(&p), &[7], who(9));
    req.status = ApprovalStatus::Cancelled;
    s.put_approval(req);
    assert!(s.live_approval_for_hash(&plan_hash(&p)).is_none());
    assert_eq!(needs_approval(&s, &p, &Auth::yes().approvers(&[7])), vec![AccountId(1)]);
}

#[test]
fn one_authorized_actor_can_never_gate_anything_and_is_refused() {
    // A11: the "one authorized actor" is the poster, so such a rule is inert. It is
    // refused at set time in E3b; the pure engine fails CLOSED if one is stored.
    let mut s = snap();
    let mut a = account(1, 1000);
    a.approval = Some(ApprovalRule {
        threshold: Amount::from_minor(100),
        policy: ApprovalPolicy::OneAuthorizedActor,
    });
    s.put_account(a);
    let p = plan("inert", who(1), vec![line(1, -500), line(2, 500)]);
    assert_eq!(refuse_with(&s, &p, &Auth::yes(), &Custody::open()), EconomyError::ApprovalRequired);
}

// ---- reversal --------------------------------------------------------------

#[test]
fn reversing_an_effect_carrying_transaction_is_refused() {
    // C5: money-only compensation would pay the buyer back AND leave them the title.
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    let mut p = plan("sale", who(1), vec![line(1, -100), line(2, 100)]);
    p.asset_ops = vec![AssetOp::TransferTitle { object: obj(50), to: who(2), to_scope: None }];
    let (s2, tx) = post(&s, &p, 1);
    match plan_reversal(&tx, who(1), EconomicCause::Staff { reason: "undo".into() }, &s2, &Auth::yes())
    {
        PlanResult::Refused(e) => assert_eq!(e, EconomyError::HasEffects),
        other => panic!("expected Refused(HasEffects), got {other:?}"),
    }
}

#[test]
fn the_reversal_path_ignores_an_approval_rule() {
    // A7: an ApprovalRule must never become a shield against clawback.
    let mut clean = snap();
    clean.put_account(Account { credit: CreditPolicy::unbounded(), ..account(2, 0) });
    let p = plan("pre", who(1), vec![line(1, -900), line(2, 900)]);
    let (posted, tx) = post(&clean, &p, 1);
    // Now put a rule on the debited account, as staff would after the fact.
    let mut s2 = posted.clone();
    let mut ruled = posted.account(AccountId(1)).unwrap().clone();
    ruled.approval = Some(rule(10, 2));
    s2.put_account(ruled);
    // A NEW spend past the threshold escalates...
    let fresh = plan("new", who(1), vec![line(1, -50), line(2, 50)]);
    assert_eq!(needs_approval(&s2, &fresh, &Auth::yes()), vec![AccountId(1)]);
    // ...but the clawback does not even evaluate invariant 17.
    match plan_reversal(&tx, who(1), EconomicCause::Staff { reason: "clawback".into() }, &s2, &Auth::yes())
    {
        PlanResult::Post(prepared) => {
            assert_eq!(prepared.consumed_approval, None);
            let (s3, rtx) = apply(&s2, &prepared, TransactionId(2), 2, NOW);
            assert_eq!(rtx.reverses, Some(tx.id));
            assert_eq!(bal(&s3, 1), 1000, "the clawback completed");
            assert!(rtx.asset_effects.is_empty() && rtx.custody_effects.is_empty());
        }
        other => panic!("expected Post, got {other:?}"),
    }
}

// ---- plan_hash v2 ----------------------------------------------------------

#[test]
fn a_money_only_plan_still_hashes_to_the_pinned_e2_digest() {
    // D-E3-10 / C7: the v2 op vectors are skip-if-empty, so every pre-E3 idempotency
    // record still resolves. This digest was computed on the E2a code.
    let mut p = EconomyPlan::new(
        IdempotencyKey::new("pin", "v2"),
        PrincipalId::Object(ObjId(7)),
        EconomicCause::Staff { reason: "pin".into() },
        vec![
            RequestedLine { account: AccountId(1), delta: Amount::from_minor(-250), memo: "out".into() },
            RequestedLine { account: AccountId(2), delta: Amount::from_minor(250), memo: "in".into() },
        ],
    );
    p.description = "hash pin".into();
    let hex: String = plan_hash(&p).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, "9c917eee84e001912480fdb8306b9a762ed2e3f50d1014e34ae38601c490d74e");
}

#[test]
fn op_order_does_not_change_the_hash_but_op_content_does() {
    let a = asset_plan("h", who(1), vec![
        AssetOp::Release { object: obj(50), encumbrance: lien(9, 5) },
        AssetOp::Assign { object: obj(50), to: who(3), purpose: "x".into(), valid_until: None },
        AssetOp::Unassign { object: obj(51) },
    ]);
    let b = asset_plan("h", who(1), vec![
        AssetOp::Unassign { object: obj(51) },
        AssetOp::Assign { object: obj(50), to: who(3), purpose: "x".into(), valid_until: None },
        AssetOp::Release { object: obj(50), encumbrance: lien(9, 5) },
    ]);
    assert_eq!(plan_hash(&a), plan_hash(&b), "the op order is canonicalized away");
    // Two Releases differing only in payload sort by canonical bytes — a total order.
    let c = asset_plan("h", who(1), vec![
        AssetOp::Release { object: obj(50), encumbrance: lien(9, 6) },
        AssetOp::Assign { object: obj(50), to: who(3), purpose: "x".into(), valid_until: None },
        AssetOp::Unassign { object: obj(51) },
    ]);
    assert_ne!(plan_hash(&a), plan_hash(&c));
    // Custody ops participate too.
    let mut d = a.clone();
    d.custody_ops = vec![CustodyOp::Deposit { object: obj(50), vault: VaultId(1) }];
    assert_ne!(plan_hash(&a), plan_hash(&d));
}

#[test]
fn applied_order_matches_hashed_order_for_two_plans_that_differ_only_in_order() {
    let mut s = snap();
    let mut a = asset(50, who(1));
    a.encumbrances = vec![lien(9, 5)];
    s.put_asset(a);
    let ops = || {
        vec![
            AssetOp::TransferTitle { object: obj(50), to: who(2), to_scope: None },
            AssetOp::Release { object: obj(50), encumbrance: lien(9, 5) },
        ]
    };
    let mut reversed_ops = ops();
    reversed_ops.reverse();
    let (s1, t1) = post(&s, &asset_plan("o1", who(1), ops()), 1);
    let (s2, t2) = post(&s, &asset_plan("o2", who(1), reversed_ops), 1);
    assert_eq!(s1.asset(obj(50)), s2.asset(obj(50)), "same post-state either way");
    let kinds = |t: &EconomicTransaction| t.asset_effects.iter().map(|e| e.kind).collect::<Vec<_>>();
    assert_eq!(kinds(&t1), kinds(&t2));
    assert_eq!(kinds(&t1), vec![AssetEffectKind::Released, AssetEffectKind::TitleTransferred]);
}

// ---- boundedness (invariant 21) -------------------------------------------

#[test]
fn the_new_caps_refuse_oversized_plans() {
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    let many: Vec<AssetOp> = (0..=MAX_ASSET_OPS_PER_PLAN)
        .map(|i| AssetOp::Unassign { object: obj(200 + i as u32) })
        .collect();
    assert!(matches!(refuse(&s, &asset_plan("many", who(1), many)), EconomyError::TooLarge(_)));
    let big = "z".repeat(MAX_REASON_BYTES + 1);
    let p = asset_plan("reason", who(1), vec![AssetOp::WriteOff {
        object: obj(50),
        reason: big.clone(),
    }]);
    assert!(matches!(refuse(&s, &p), EconomyError::TooLarge(_)));
    let p = asset_plan("purpose", who(1), vec![AssetOp::Assign {
        object: obj(50),
        to: who(3),
        purpose: big.clone().into(),
        valid_until: None,
    }]);
    assert!(matches!(refuse(&s, &p), EconomyError::TooLarge(_)));
    let p = asset_plan("dispute", who(1), vec![AssetOp::Encumber {
        object: obj(50),
        encumbrance: disputed(8, &big),
    }]);
    assert!(matches!(refuse(&s, &p), EconomyError::TooLarge(_)));
    // Too many custody ops.
    let mut c = plan("cust", who(1), Vec::new());
    c.custody_ops = (0..=mush_economy::MAX_CUSTODY_OPS_PER_PLAN)
        .map(|i| CustodyOp::Deposit { object: obj(300 + i as u32), vault: VaultId(1) })
        .collect();
    assert!(matches!(refuse(&s, &c), EconomyError::TooLarge(_)));
}

#[test]
fn an_asset_cannot_carry_more_than_the_encumbrance_cap() {
    let mut s = snap();
    let mut a = asset(50, who(1));
    a.encumbrances = (0..MAX_ENCUMBRANCES_PER_ASSET as i64).map(|i| lien(9, i)).collect();
    s.put_asset(a);
    let p = asset_plan("cap", who(1), vec![AssetOp::Encumber {
        object: obj(50),
        encumbrance: lien(9, 999),
    }]);
    assert!(matches!(refuse(&s, &p), EconomyError::TooLarge(_)));
}

#[test]
fn the_entry_probe_covers_the_effects() {
    // Invariant 21: an accepted plan provably fits MAX_ENTRY_BYTES *with* its
    // effects, which are by far the largest part of an E3 entry (each one carries
    // the asset's whole post-state, claims and all).
    let mut s = snap();
    for i in 0..MAX_ASSET_OPS_PER_PLAN as u32 {
        let mut a = asset(400 + i, who(1));
        a.encumbrances = (0..MAX_ENCUMBRANCES_PER_ASSET)
            .map(|n| disputed(8, &format!("{n}{}", "d".repeat(MAX_REASON_BYTES))))
            .collect();
        s.put_asset(a);
    }
    let ops: Vec<AssetOp> = (0..MAX_ASSET_OPS_PER_PLAN as u32)
        .map(|i| AssetOp::Assign {
            object: obj(400 + i),
            to: who(3),
            purpose: "p".repeat(MAX_REASON_BYTES).into(),
            valid_until: None,
        })
        .collect();
    let p = asset_plan("huge", who(1), ops);
    assert!(matches!(refuse(&s, &p), EconomyError::TooLarge(_)));
    // The same plan over ONE asset fits, so the cap is measuring, not just counting.
    let small = asset_plan("small", who(1), vec![AssetOp::Assign {
        object: obj(400),
        to: who(3),
        purpose: "p".repeat(MAX_REASON_BYTES).into(),
        valid_until: None,
    }]);
    assert!(validate(&small, &s, &env(&Auth::yes(), &NoCustody)).is_post());
}

// ---- vocabulary + snapshot indexes ----------------------------------------

#[test]
fn the_capability_vocabulary_is_exported_and_complete() {
    for cap in [
        "economy.account.post",
        "economy.account.view",
        "economy.account.approve",
        "asset.title.transfer",
        "asset.assign",
        "vault.view",
        "vault.deposit",
        "vault.withdraw",
        // E4b2: an org contract's four verbs, checked on the contract's own scope.
        "contract.draft",
        "contract.accept",
        "contract.attest",
        "contract.dispute",
    ] {
        assert!(CAPABILITIES.contains(&cap), "{cap} missing from the vocabulary");
        assert!(mush_org::CapabilityId::new(cap).is_some(), "{cap} is not a valid capability id");
    }
    assert_eq!(CAPABILITIES.len(), 12);
}

#[test]
fn the_derived_indexes_follow_the_records() {
    let mut s = snap();
    s.put_asset(asset(50, who(1)));
    s.put_asset(asset(51, who(1)));
    assert_eq!(s.assets_of(&who(1)).count(), 2);
    s.put_vault(vault(1, 80));
    assert_eq!(s.vault_by_backing(&CustodyRef::Container(obj(80))).map(|v| v.id), Some(VaultId(1)));
    assert!(s.is_vault_backing(obj(80)));
    assert!(!s.is_vault_backing(obj(81)));
    // A title transfer moves the asset between owner buckets.
    let p = asset_plan("mv", who(1), vec![AssetOp::TransferTitle {
        object: obj(50),
        to: who(2),
        to_scope: None,
    }]);
    let (s2, _) = post(&s, &p, 1);
    assert_eq!(s2.assets_of(&who(1)).count(), 1);
    assert_eq!(s2.assets_of(&who(2)).map(|a| a.object).collect::<Vec<_>>(), vec![obj(50)]);
}

// ---- persisted shape -------------------------------------------------------

#[test]
fn an_e2_shaped_account_and_journal_entry_still_deserialize() {
    // The whole point of `#[serde(default)]` on the three new transaction fields and
    // on `Account.approval`: E2b boot parses every stored entry, and a pre-upgrade
    // journal must replay unchanged under the E3 binary.
    let acct: Account = serde_json::from_value(serde_json::json!({
        "id": 1, "owner": { "Object": 1 }, "instrument": "gold", "scope": null,
        "name": "a1", "status": "Active", "posted_balance": 500, "held_amount": 0,
        "credit": { "allow_negative": false, "floor": 0 }, "last_journal_seq": 3, "revision": 2
    }))
    .expect("an E2 account doc still parses");
    assert_eq!(acct.approval, None);

    let tx: EconomicTransaction = serde_json::from_value(serde_json::json!({
        "id": 0, "journal_seq": 0, "idempotency": { "namespace": "x", "key": "0" },
        "cause": { "Staff": { "reason": "old" } }, "actor": { "Object": 1 }, "posted_at": 0,
        "description": "", "lines": [
            { "account": 1, "instrument": "gold", "delta": -5, "memo": "" },
            { "account": 2, "instrument": "gold", "delta": 5, "memo": "" }
        ], "reverses": null, "visibility": "Parties", "plan_hash": vec![0u8; 32]
    }))
    .expect("an E2 journal entry still parses");
    assert!(tx.asset_effects.is_empty());
    assert!(tx.custody_effects.is_empty());
    assert_eq!(tx.consumed_approval, None);
}

#[test]
fn the_asset_latch_defaults_and_encumbrances_are_serde_tagged() {
    // An asset doc written before the latch existed reads as un-frozen; the tagged
    // encumbrance shape is what lets E4 add Lease/Collateral without a migration.
    let a: EconomicAsset = serde_json::from_value(serde_json::json!({
        "object": 50, "title_owner": { "Object": 1 }, "scope": null, "assignment": null,
        "maintenance_account": null, "catalog": null,
        "encumbrances": [{ "kind": "Disputed", "claimant": { "Object": 8 }, "reason": "r" }],
        "acquisition": null, "lifecycle": "Active", "last_journal_seq": 1, "revision": 1
    }))
    .expect("an asset doc without the latch parses");
    assert!(!a.frozen);
    assert_eq!(a.frozen_reason, None);
    assert_eq!(a.encumbrances, vec![disputed(8, "r")]);
    let json = serde_json::to_value(lien(9, 5)).unwrap();
    assert_eq!(json["kind"], "Lien", "the claim carries its discriminant as a field");
}

// ---- fuzz ------------------------------------------------------------------

#[test]
fn fuzz_random_op_sequences_never_panic_and_never_post_an_empty_transaction() {
    // Deterministic pseudo-random plans mixing money, title, and custody. Nothing
    // panics; every ACCEPTED post has ≥2 money lines OR ≥1 effect (invariant 10),
    // per-instrument conservation holds, and no asset revision ever goes backwards.
    let mut s = snap();
    s.put_account(Account { credit: CreditPolicy::unbounded(), ..account(2, 0) });
    s.put_vault(vault(1, 80));
    for i in 0..4u32 {
        s.put_asset(asset(60 + i, who(1)));
    }
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let auth = Auth::yes();
    let custody = Custody::open().holding(&[60, 61, 62, 63]);
    let mut revisions: BTreeMap<ObjId, u64> = BTreeMap::new();
    let mut txid = 1u64;
    for step in 0..1500u64 {
        let object = obj(60 + (next() % 4) as u32);
        let mut p = plan(&format!("k{step}"), who(1), Vec::new());
        if next() % 2 == 0 {
            let amt = (next() % 400) as i64 + 1;
            p.requested_lines = vec![line(1, -amt), line(2, amt)];
        }
        match next() % 8 {
            0 => p.asset_ops = vec![AssetOp::Unassign { object }],
            1 => {
                p.asset_ops = vec![AssetOp::Assign {
                    object,
                    to: who(3),
                    purpose: "f".into(),
                    valid_until: None,
                }];
            }
            2 => p.asset_ops = vec![AssetOp::TransferTitle { object, to: who(2), to_scope: None }],
            3 => p.asset_ops = vec![AssetOp::WriteOff { object, reason: "f".into() }],
            4 => p.asset_ops = vec![AssetOp::Encumber { object, encumbrance: lien(9, 1) }],
            5 => p.asset_ops = vec![AssetOp::Release { object, encumbrance: lien(9, 1) }],
            6 => p.custody_ops = vec![CustodyOp::Deposit { object, vault: VaultId(1) }],
            _ => {
                p.custody_ops = vec![CustodyOp::Withdraw { object, vault: VaultId(1), to: obj(4) }];
            }
        }
        match validate(&p, &s, &env(&auth, &custody)) {
            PlanResult::Post(prepared) => {
                assert!(
                    prepared.lines.len() >= 2
                        || !prepared.asset_effects.is_empty()
                        || !prepared.custody_effects.is_empty(),
                    "invariant 10: an accepted transaction is never empty"
                );
                let (s2, tx) = apply(&s, &prepared, TransactionId(txid), txid, NOW);
                for e in &tx.asset_effects {
                    let prev = revisions.insert(e.object, e.post.revision).unwrap_or(0);
                    assert!(e.post.revision > prev, "asset revisions only ever advance");
                }
                s = s2;
                txid += 1;
            }
            PlanResult::Refused(_) | PlanResult::AlreadyPosted(_) | PlanResult::NeedsApproval(_) => {}
        }
        let sum: i128 = s.accounts().map(|a| a.posted_balance.widen()).sum();
        assert_eq!(sum, 1000, "gold conservation broke at step {step}");
    }
}
