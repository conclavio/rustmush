//! E4a acceptance — the pure holds / clocks / obligations / contracts engine on
//! hand-built snapshots, in the E2a/E3a rhythm: every hold op and refusal, the
//! available-balance invariant, the catch-up arithmetic, the contract consent
//! surface and its deterministic split, `plan_hash` v3 (including the hard-coded
//! E3-shaped digest pin), and a shape fuzz.
//!
//! Everything here is world-free. The live seams are fakes: an [`AuthEval`] whose
//! every answer is a switch, a [`CustodyEval`] standing in for world containment,
//! and a [`FactEval`] standing in for E4c's provider.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use mush_economy::{
    Acceptance, Account, AccountId, AccountStatus, Amount, ApprovalRule, AssetLifecycle, AssetOp,
    Attestation, AttestorRef, AttestorSet, AuthEval, CadenceClock, CadenceSource, Clocks,
    ConsentSurface, Consideration, ContractAuth, ContractError, ContractId, ContractState,
    ContractTerms, CreditPolicy, CustodyEval, CustodyOp, CustodyRef, Deliverable, EconomicAsset,
    EconomicCause, EconomicClock, EconomicContract, EconomicDeadline, EconomicSchedule,
    EconomicTransaction, EconomicVisibility, EconomyError, EconomyPlan, EconomySnapshot,
    Encumbrance, Expiry, FactEval, ForgiveCause, GracePolicy, Hold, HoldEffectKind, HoldId, HoldOp,
    HoldPurpose, HoldRevision, HoldStatus, IdempotencyKey, InstrumentDef, InstrumentId,
    InstrumentKind, InstrumentStatus, MAX_ACTIVE_HOLDS_PER_ACCOUNT, MAX_CATCH_UP_CAP,
    MAX_HOLD_OPS_PER_PLAN, NegativePolicy, Obligation, ObligationCause, ObligationError,
    ObligationEvent, ObligationStatus, PayoutShare, PlanResult, Prepared, RequestedLine,
    SettleOutcome, Share, SweepIntent, TransactionId, TypedCondition, ValidateEnv, Vault, VaultId,
    VaultStatus, advance, advance_cadence, advance_obligation, apply, check_offerable,
    classify_forgiveness, consent_hash, due_periods, forgive, forgive_all, obligation_key,
    plan_hash, plan_reversal, plan_sweep, refund_plan, reverify, settle_key, settlement_plan,
    validate,
};
use mush_org::{ApprovalPolicy, CapabilityId, PrincipalId};

// ---- builders -------------------------------------------------------------

const NOW: i64 = 10_000;
const IC: i64 = 500_000;

fn iid(s: &str) -> InstrumentId {
    InstrumentId::new(s).unwrap()
}
fn obj(n: u32) -> ObjId {
    ObjId(n)
}
fn who(n: u32) -> PrincipalId {
    PrincipalId::Object(ObjId(n))
}
fn key(k: &str) -> IdempotencyKey {
    IdempotencyKey::new("e4a", k)
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
        display: None,
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
        credit: CreditPolicy::forbid(),
        last_journal_seq: 0,
        revision: 1,
        approval: None,
        last_hold_seq: 0,
    }
}
fn hold(id: u64, account: u64, amount: i64) -> Hold {
    Hold {
        id: HoldId(id),
        account: AccountId(account),
        amount: Amount::from_minor(amount),
        beneficiary: None,
        purpose: HoldPurpose::Manual {
            reason: "test".into(),
        },
        placed_by: who(1),
        holder: key("placed"),
        expires: None,
        status: HoldStatus::Held,
        last_journal_seq: 0,
        revision: 1,
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
        stolen: None,
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
/// Accounts #1 = 1000, #2 = 0, #9 = 0 over one instrument.
fn snap() -> EconomySnapshot {
    let mut s = EconomySnapshot::new();
    s.put_instrument(instrument("gold"));
    s.put_account(account(1, 1000));
    s.put_account(account(2, 0));
    s.put_account(account(9, 0));
    s
}
/// Put a hold into the snapshot AND fold it into the account projection, the way
/// `apply` would have.
fn with_hold(s: &mut EconomySnapshot, h: Hold) {
    let account = s.account(h.account).expect("account exists").clone();
    let held = account.held_amount.checked_add(h.amount).expect("fits");
    s.put_account(Account {
        held_amount: held,
        ..account
    });
    s.put_hold(h);
}
fn line(account: u64, delta: i64) -> RequestedLine {
    RequestedLine {
        account: AccountId(account),
        delta: Amount::from_minor(delta),
        memo: "".into(),
    }
}
fn plan(k: &str, actor: PrincipalId, lines: Vec<RequestedLine>) -> EconomyPlan {
    EconomyPlan::new(
        key(k),
        actor,
        EconomicCause::Staff {
            reason: "test".into(),
        },
        lines,
    )
}
fn hold_plan(k: &str, ops: Vec<HoldOp>) -> EconomyPlan {
    let mut p = plan(k, who(1), Vec::new());
    p.hold_ops = ops;
    p
}
fn place(account: u64, amount: i64) -> HoldOp {
    HoldOp::Place {
        account: AccountId(account),
        amount: Amount::from_minor(amount),
        beneficiary: None,
        purpose: HoldPurpose::Manual { reason: "r".into() },
        expires: None,
    }
}

// ---- the seams as fakes ---------------------------------------------------

#[derive(Clone)]
struct Auth {
    money: bool,
    hold: bool,
    asset: bool,
    receipt: bool,
    attestor: bool,
    approvers: BTreeSet<ObjId>,
}
impl Auth {
    fn yes() -> Auth {
        Auth {
            money: true,
            hold: true,
            asset: true,
            receipt: true,
            attestor: true,
            approvers: BTreeSet::new(),
        }
    }
}
impl AuthEval for Auth {
    fn authorize(&self, _: &PrincipalId, _: &Account, _: Amount) -> bool {
        self.money
    }
    fn authorize_asset(&self, _: &PrincipalId, _: Option<&EconomicAsset>, _: &AssetOp) -> bool {
        self.asset
    }
    fn authorize_title_receipt(
        &self,
        _: &PrincipalId,
        _: &PrincipalId,
        _: Option<mush_org::OrgScopeId>,
    ) -> bool {
        self.receipt
    }
    fn authorize_vault(&self, _: &PrincipalId, _: &Vault, _: &CustodyOp) -> bool {
        true
    }
    fn authorize_approver(&self, a: ObjId, _: &Account, _: &ApprovalPolicy) -> bool {
        self.approvers.contains(&a)
    }
    fn authorize_hold(&self, _: &PrincipalId, _: &Hold, _: &HoldOp) -> bool {
        self.hold
    }
    fn authorize_attestor(&self, _: &PrincipalId, _: &AttestorSet) -> bool {
        self.attestor
    }
}

#[derive(Clone, Default)]
struct Custody {
    held: BTreeSet<ObjId>,
}
impl Custody {
    fn holding(ids: &[u32]) -> Custody {
        Custody {
            held: ids.iter().map(|n| obj(*n)).collect(),
        }
    }
}
impl CustodyEval for Custody {
    fn backing_valid(&self, _: &Vault) -> bool {
        true
    }
    fn holds(&self, _: &Vault, o: ObjId) -> bool {
        self.held.contains(&o)
    }
    fn may_deposit(&self, _: ObjId, _: &PrincipalId) -> bool {
        true
    }
    fn destination_ok(&self, _: &PrincipalId, _: ObjId) -> bool {
        true
    }
}

#[derive(Clone, Default)]
struct Facts {
    answer: Option<bool>,
}
impl FactEval for Facts {
    fn resolve(&self, _: &mush_economy::ExternalRef) -> Option<bool> {
        self.answer
    }
}

// ---- drivers --------------------------------------------------------------

struct Seams {
    auth: Auth,
    custody: Custody,
    facts: Facts,
    now: i64,
    ic_now: i64,
}
impl Seams {
    fn open() -> Seams {
        Seams {
            auth: Auth::yes(),
            custody: Custody::default(),
            facts: Facts::default(),
            now: NOW,
            ic_now: IC,
        }
    }
    fn env<'a>(&'a self, s: &EconomySnapshot) -> ValidateEnv<'a> {
        ValidateEnv {
            keys: &mush_economy::MapOnlyKeys,
            auth: &self.auth,
            custody: &self.custody,
            facts: &self.facts,
            now: self.now,
            ic_now: self.ic_now,
            next_hold_id: s.next_hold_floor(),
            spent: std::collections::BTreeMap::new(),
        }
    }
}

fn prepare(s: &EconomySnapshot, p: &EconomyPlan, k: &Seams) -> Prepared {
    match validate(p, s, &k.env(s)) {
        PlanResult::Post(prepared) => *prepared,
        other => panic!("expected Post, got {other:?}"),
    }
}
fn post_with(
    s: &EconomySnapshot,
    p: &EconomyPlan,
    txid: u64,
    k: &Seams,
) -> (EconomySnapshot, EconomicTransaction) {
    let prepared = prepare(s, p, k);
    apply(s, &prepared, TransactionId(txid), txid + 100, NOW)
}
fn post(s: &EconomySnapshot, p: &EconomyPlan, txid: u64) -> (EconomySnapshot, EconomicTransaction) {
    post_with(s, p, txid, &Seams::open())
}
fn refuse_with(s: &EconomySnapshot, p: &EconomyPlan, k: &Seams) -> EconomyError {
    match validate(p, s, &k.env(s)) {
        PlanResult::Refused(e) => e,
        other => panic!("expected Refused, got {other:?}"),
    }
}
fn refuse(s: &EconomySnapshot, p: &EconomyPlan) -> EconomyError {
    refuse_with(s, p, &Seams::open())
}

// ===========================================================================
// Holds — the ops
// ===========================================================================

#[test]
fn placing_a_hold_reserves_available_funds_without_moving_the_posted_balance() {
    let s = snap();
    let (s2, tx) = post(&s, &hold_plan("place", vec![place(1, 250)]), 1);
    assert_eq!(
        tx.lines.len(),
        0,
        "a placement is an effect-only entry (invariant 10)"
    );
    assert_eq!(tx.hold_effects.len(), 1);
    assert_eq!(tx.hold_effects[0].kind, HoldEffectKind::Placed);
    assert_eq!(tx.hold_effects[0].hold.status, HoldStatus::Held);
    let a = s2.account(AccountId(1)).unwrap();
    assert_eq!(
        a.posted_balance,
        Amount::from_minor(1000),
        "posted is untouched"
    );
    assert_eq!(a.held_amount, Amount::from_minor(250));
    assert_eq!(a.available(), Some(Amount::from_minor(750)));
    assert_eq!(
        a.last_journal_seq, 0,
        "an effect-only entry never moves the money anchor"
    );
    assert_eq!(a.last_hold_seq, 101, "held_amount has its OWN anchor (D11)");
}

#[test]
fn the_placing_plans_key_is_recorded_as_the_holder() {
    let s = snap();
    let (s2, _) = post(&s, &hold_plan("holder", vec![place(1, 10)]), 1);
    let h = s2.holds().next().unwrap();
    assert_eq!(
        h.holder,
        key("holder"),
        "doc 09 C8's holder is the placing plan's key"
    );
    assert_eq!(h.placed_by, who(1));
}

#[test]
fn a_capture_takes_its_explicit_amount_and_releases_the_remainder() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 400));
    let mut p = plan("cap", who(1), vec![line(1, -100), line(2, 100)]);
    p.hold_ops = vec![HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::from_minor(100),
    }];
    let (s2, tx) = post(&s, &p, 7);
    assert_eq!(tx.hold_effects[0].kind, HoldEffectKind::Captured);
    assert_eq!(
        tx.hold_effects[0].hold.status,
        HoldStatus::Consumed {
            transaction: TransactionId(7),
            amount: Amount::from_minor(100)
        },
        "apply stamps the consuming transaction, exactly like an acquisition"
    );
    let a = s2.account(AccountId(1)).unwrap();
    assert_eq!(a.posted_balance, Amount::from_minor(900));
    assert_eq!(
        a.held_amount,
        Amount::ZERO,
        "the uncaptured 300 was released with it"
    );
    assert_eq!(a.available(), Some(Amount::from_minor(900)));
}

#[test]
fn two_captures_on_one_account_attribute_per_hold() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 400));
    with_hold(&mut s, hold(2, 1, 300));
    let mut p = plan("two", who(1), vec![line(1, -250), line(2, 250)]);
    p.hold_ops = vec![
        HoldOp::Capture {
            hold: HoldId(1),
            amount: Amount::from_minor(100),
        },
        HoldOp::Capture {
            hold: HoldId(2),
            amount: Amount::from_minor(150),
        },
    ];
    let (s2, tx) = post(&s, &p, 3);
    let mut captured: BTreeMap<u64, i64> = BTreeMap::new();
    for e in &tx.hold_effects {
        if let HoldStatus::Consumed { amount, .. } = e.hold.status {
            captured.insert(e.hold.id.0, amount.minor());
        }
    }
    assert_eq!(
        captured,
        BTreeMap::from([(1, 100), (2, 150)]),
        "each effect records its OWN capture"
    );
    let a = s2.account(AccountId(1)).unwrap();
    assert_eq!(a.held_amount, Amount::ZERO);
    assert_eq!(a.posted_balance, Amount::from_minor(750));
}

#[test]
fn a_release_frees_the_reservation_and_leaves_the_balance_alone() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 400));
    let (s2, tx) = post(
        &s,
        &hold_plan("rel", vec![HoldOp::Release { hold: HoldId(1) }]),
        2,
    );
    assert_eq!(tx.hold_effects[0].kind, HoldEffectKind::Released);
    assert_eq!(tx.hold_effects[0].hold.status, HoldStatus::Released);
    let a = s2.account(AccountId(1)).unwrap();
    assert_eq!(a.held_amount, Amount::ZERO);
    assert_eq!(a.posted_balance, Amount::from_minor(1000));
}

#[test]
fn expiry_needs_the_sweeps_cause_and_a_deadline_that_has_passed() {
    let mut s = snap();
    with_hold(
        &mut s,
        Hold {
            expires: Some(EconomicDeadline::real(NOW - 1)),
            ..hold(1, 1, 400)
        },
    );
    let sweep = |k: &str| {
        let mut p = hold_plan(k, vec![HoldOp::Expire { hold: HoldId(1) }]);
        p.cause = EconomicCause::Internal {
            kind: "hold.expire".into(),
            id: 1,
        };
        p
    };
    let (s2, tx) = post(&s, &sweep("exp"), 4);
    assert_eq!(tx.hold_effects[0].hold.status, HoldStatus::Expired);
    assert_eq!(s2.account(AccountId(1)).unwrap().held_amount, Amount::ZERO);

    // A player-caused plan may never age a hold out early.
    let staff = hold_plan("staff-exp", vec![HoldOp::Expire { hold: HoldId(1) }]);
    assert_eq!(refuse(&s, &staff), EconomyError::HoldExpireNotSweep);
}

#[test]
fn an_unexpired_or_deadlineless_hold_refuses_to_expire() {
    let mut future = snap();
    with_hold(
        &mut future,
        Hold {
            expires: Some(EconomicDeadline::real(NOW + 1)),
            ..hold(1, 1, 5)
        },
    );
    let mut none = snap();
    with_hold(&mut none, hold(1, 1, 5));
    let mut p = hold_plan("e", vec![HoldOp::Expire { hold: HoldId(1) }]);
    p.cause = EconomicCause::Internal {
        kind: "hold.expire".into(),
        id: 1,
    };
    assert_eq!(refuse(&future, &p), EconomyError::HoldNotExpired(HoldId(1)));
    assert_eq!(refuse(&none, &p), EconomyError::HoldNotExpired(HoldId(1)));
}

#[test]
fn a_manual_deadline_never_expires_on_someone_elses_clock() {
    let mut s = snap();
    let deadline = EconomicDeadline {
        clock: EconomicClock::Manual,
        at: 0,
    };
    with_hold(
        &mut s,
        Hold {
            expires: Some(deadline),
            ..hold(1, 1, 5)
        },
    );
    let mut p = hold_plan("m", vec![HoldOp::Expire { hold: HoldId(1) }]);
    p.cause = EconomicCause::Internal {
        kind: "hold.expire".into(),
        id: 1,
    };
    assert_eq!(refuse(&s, &p), EconomyError::HoldNotExpired(HoldId(1)));
}

#[test]
fn a_cadence_deadline_with_no_record_is_never_passed() {
    let mut s = snap();
    let deadline = EconomicDeadline {
        clock: EconomicClock::Cadence(mush_economy::CadenceId(4)),
        at: 0,
    };
    with_hold(
        &mut s,
        Hold {
            expires: Some(deadline),
            ..hold(1, 1, 5)
        },
    );
    let mut p = hold_plan("c", vec![HoldOp::Expire { hold: HoldId(1) }]);
    p.cause = EconomicCause::Internal {
        kind: "hold.expire".into(),
        id: 1,
    };
    assert_eq!(refuse(&s, &p), EconomyError::HoldNotExpired(HoldId(1)));

    // With the record present and the position past the deadline, it expires.
    s.put_cadence(CadenceClock {
        id: mush_economy::CadenceId(4),
        name: "season".into(),
        source: CadenceSource::RealUtc,
        period: 100,
        catch_up_cap: 3,
        position: 5,
        advanced_at: 0,
        revision: 1,
        generation: mush_economy::CADENCE_GENERATION_INITIAL,
        schema: mush_economy::CADENCE_SCHEMA_LEGACY,
    });
    assert!(validate(&p, &s, &Seams::open().env(&s)).is_post());
}

#[test]
fn every_op_on_a_terminal_hold_is_refused() {
    for status in [
        HoldStatus::Released,
        HoldStatus::Expired,
        HoldStatus::Consumed {
            transaction: TransactionId(1),
            amount: Amount::from_minor(1),
        },
    ] {
        let mut s = snap();
        s.put_hold(Hold {
            status: status.clone(),
            ..hold(1, 1, 100)
        });
        let ops = [
            HoldOp::Capture {
                hold: HoldId(1),
                amount: Amount::from_minor(1),
            },
            HoldOp::Release { hold: HoldId(1) },
        ];
        for op in ops {
            assert_eq!(
                refuse(&s, &hold_plan("t", vec![op])),
                EconomyError::HoldNotHeld(HoldId(1)),
                "{status:?}"
            );
        }
    }
}

#[test]
fn an_unknown_hold_is_refused_by_id() {
    let s = snap();
    assert_eq!(
        refuse(
            &s,
            &hold_plan("u", vec![HoldOp::Release { hold: HoldId(77) }])
        ),
        EconomyError::NoSuchHold(HoldId(77))
    );
}

#[test]
fn a_capture_over_the_holds_amount_is_refused() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 100));
    let over = HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::from_minor(101),
    };
    assert_eq!(
        refuse(&s, &hold_plan("o", vec![over])),
        EconomyError::CaptureExceedsHold(HoldId(1))
    );
    let zero = HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::ZERO,
    };
    assert_eq!(
        refuse(&s, &hold_plan("z", vec![zero])),
        EconomyError::CaptureExceedsHold(HoldId(1))
    );
}

#[test]
fn a_non_positive_placement_is_refused() {
    let s = snap();
    assert_eq!(
        refuse(&s, &hold_plan("p0", vec![place(1, 0)])),
        EconomyError::HoldAmountNotPositive
    );
    assert_eq!(
        refuse(&s, &hold_plan("pn", vec![place(1, -5)])),
        EconomyError::HoldAmountNotPositive
    );
}

#[test]
fn a_placement_on_an_unbounded_credit_account_is_refused() {
    let mut s = snap();
    s.put_account(Account {
        credit: CreditPolicy::unbounded(),
        ..account(1, 1000)
    });
    assert_eq!(
        refuse(&s, &hold_plan("ub", vec![place(1, 10)])),
        EconomyError::HoldOnUnboundedAccount(AccountId(1)),
        "permits() is vacuous there, so the reservation would reserve nothing"
    );
}

#[test]
fn a_placement_on_a_non_active_account_is_refused() {
    let mut s = snap();
    s.put_account(Account {
        status: AccountStatus::Frozen,
        ..account(1, 1000)
    });
    assert_eq!(
        refuse(&s, &hold_plan("fr", vec![place(1, 10)])),
        EconomyError::InactiveAccount(AccountId(1))
    );
}

#[test]
fn a_placement_needs_debit_authority_asked_as_a_negative_probe() {
    let s = snap();
    let mut k = Seams::open();
    k.auth.money = false;
    assert_eq!(
        refuse_with(&s, &hold_plan("na", vec![place(1, 10)]), &k),
        EconomyError::Unauthorized
    );
}

#[test]
fn a_release_needs_the_hold_seam_and_never_plain_money_authority() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 100));
    let mut k = Seams::open();
    k.auth.hold = false;
    assert_eq!(
        refuse_with(
            &s,
            &hold_plan("nr", vec![HoldOp::Release { hold: HoldId(1) }]),
            &k
        ),
        EconomyError::Unauthorized,
        "money authority is not a licence to free someone else's claim"
    );
}

#[test]
fn a_capture_needs_the_hold_seam_and_never_plain_money_authority() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 100));
    let mut p = plan("cf", who(1), vec![line(1, -40), line(2, 40)]);
    p.hold_ops = vec![HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::from_minor(40),
    }];
    // Invariant 25, as the E4a verify fold rewrote it (finding 1): the hold seam is
    // MANDATORY and there is no fallback to debit authority on the account. A hold
    // stands between an account's owner and their own money on someone else's
    // behalf — if the owner's wallet rights spent it, escrow would be a promise only
    // the payer could break.
    let mut money_only = Seams::open();
    money_only.auth.hold = false;
    assert_eq!(refuse_with(&s, &p, &money_only), EconomyError::Unauthorized);
    assert!(validate(&p, &s, &Seams::open().env(&s)).is_post());
}

#[test]
fn two_ops_on_one_hold_in_one_plan_are_refused() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 100));
    let p = hold_plan(
        "dup",
        vec![
            HoldOp::Release { hold: HoldId(1) },
            HoldOp::Capture {
                hold: HoldId(1),
                amount: Amount::from_minor(1),
            },
        ],
    );
    assert_eq!(refuse(&s, &p), EconomyError::DuplicateHoldOp(HoldId(1)));
}

// ===========================================================================
// Invariant 23 — available balance (scenario 13, pure half)
// ===========================================================================

#[test]
fn scenario_13_two_holds_that_together_exceed_available_refuse_the_second() {
    let s = snap();
    let (s2, _) = post(&s, &hold_plan("h1", vec![place(1, 600)]), 1);
    assert_eq!(
        s2.account(AccountId(1)).unwrap().available(),
        Some(Amount::from_minor(400))
    );
    assert_eq!(
        refuse(&s2, &hold_plan("h2", vec![place(1, 600)])),
        EconomyError::InsufficientAvailable(AccountId(1))
    );
    // …and the same two placements in ONE plan are refused the same way.
    assert_eq!(
        refuse(&s, &hold_plan("both", vec![place(1, 600), place(1, 600)])),
        EconomyError::InsufficientAvailable(AccountId(1))
    );
}

#[test]
fn a_debit_that_ignores_a_hold_is_refused_even_though_the_posted_balance_covers_it() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 900));
    let p = plan("ignore", who(1), vec![line(1, -200), line(2, 200)]);
    assert_eq!(
        refuse(&s, &p),
        EconomyError::InsufficientAvailable(AccountId(1)),
        "the money is there and it is spoken for — a different answer to CreditViolation"
    );
    // 100 still fits inside available.
    let ok = plan("fits", who(1), vec![line(1, -100), line(2, 100)]);
    assert!(validate(&ok, &s, &Seams::open().env(&s)).is_post());
}

#[test]
fn a_plain_overdraft_is_still_a_credit_violation_not_an_availability_one() {
    let s = snap();
    let p = plan("over", who(1), vec![line(1, -2000), line(2, 2000)]);
    assert_eq!(refuse(&s, &p), EconomyError::CreditViolation(AccountId(1)));
}

#[test]
fn a_capture_frees_its_own_reservation_so_the_settling_debit_fits() {
    // The whole hold is 1000 — every penny of the account — and the plan debits 1000.
    // Without invariant 23's "a capture releases what it consumes" rule this would be
    // refused as unavailable, which would make escrow unspendable.
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 1000));
    let mut p = plan("settle", who(1), vec![line(1, -1000), line(2, 1000)]);
    p.hold_ops = vec![HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::from_minor(1000),
    }];
    let (s2, _) = post(&s, &p, 1);
    assert_eq!(
        s2.account(AccountId(1)).unwrap().posted_balance,
        Amount::ZERO
    );
    assert_eq!(s2.account(AccountId(1)).unwrap().held_amount, Amount::ZERO);
}

#[test]
fn a_credit_line_is_not_a_licence_to_reserve_headroom() {
    let mut s = snap();
    s.put_account(Account {
        credit: CreditPolicy {
            allow_negative: true,
            floor: Some(Amount::from_minor(-500)),
        },
        ..account(1, 0)
    });
    // The account may go to -500, so the DEBIT would be permitted…
    let spend = plan("spend", who(1), vec![line(1, -300), line(2, 300)]);
    assert!(validate(&spend, &s, &Seams::open().env(&s)).is_post());
    // …but a reservation may not promise money that is not there (invariant 26).
    assert_eq!(
        refuse(&s, &hold_plan("res", vec![place(1, 300)])),
        EconomyError::InsufficientAvailable(AccountId(1))
    );
}

#[test]
fn the_per_account_hold_cap_is_enforced() {
    let mut s = snap();
    for i in 0..MAX_ACTIVE_HOLDS_PER_ACCOUNT {
        with_hold(&mut s, hold(i as u64 + 1, 1, 1));
    }
    assert_eq!(
        s.active_holds_on(AccountId(1)),
        MAX_ACTIVE_HOLDS_PER_ACCOUNT
    );
    assert!(matches!(
        refuse(&s, &hold_plan("cap", vec![place(1, 1)])),
        EconomyError::TooLarge(_)
    ));
}

#[test]
fn the_per_plan_hold_op_cap_is_enforced() {
    let s = snap();
    let ops: Vec<HoldOp> = (0..=MAX_HOLD_OPS_PER_PLAN).map(|_| place(1, 1)).collect();
    assert!(matches!(
        refuse(&s, &hold_plan("many", ops)),
        EconomyError::TooLarge(_)
    ));
}

#[test]
fn a_stale_expected_hold_revision_is_refused() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 100));
    let mut p = hold_plan("stale", vec![HoldOp::Release { hold: HoldId(1) }]);
    p.expected_hold_revisions = vec![HoldRevision {
        hold: HoldId(1),
        revision: 2,
    }];
    assert_eq!(
        refuse(&s, &p),
        EconomyError::StaleHold {
            hold: HoldId(1),
            expected: 2,
            found: Some(1)
        }
    );
    p.expected_hold_revisions = vec![HoldRevision {
        hold: HoldId(1),
        revision: 1,
    }];
    assert!(validate(&p, &s, &Seams::open().env(&s)).is_post());
}

#[test]
fn a_large_placement_trips_the_accounts_approval_rule() {
    let mut s = snap();
    s.put_account(Account {
        approval: Some(ApprovalRule {
            threshold: Amount::from_minor(100),
            policy: ApprovalPolicy::Threshold {
                approvals: 1,
                from_role: None,
            },
            window: None,
        }),
        ..account(1, 1000)
    });
    // D-E4-11: reserving a fortune is a debit-authority plan and escalates like one.
    match validate(
        &hold_plan("big", vec![place(1, 500)]),
        &s,
        &Seams::open().env(&s),
    ) {
        PlanResult::NeedsApproval(need) => assert_eq!(need.accounts, vec![AccountId(1)]),
        other => panic!("expected NeedsApproval, got {other:?}"),
    }
    assert!(
        validate(
            &hold_plan("small", vec![place(1, 50)]),
            &s,
            &Seams::open().env(&s)
        )
        .is_post()
    );
}

#[test]
fn invariant_28_a_capture_carrying_transaction_cannot_be_reversed() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 400));
    let mut p = plan("cap", who(1), vec![line(1, -100), line(2, 100)]);
    p.hold_ops = vec![HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::from_minor(100),
    }];
    let (s2, tx) = post(&s, &p, 7);
    let out = plan_reversal(
        &tx,
        who(1),
        EconomicCause::Staff {
            reason: "oops".into(),
        },
        &s2,
        &Auth::yes(),
        &mush_economy::MapOnlyKeys,
    );
    assert_eq!(out, PlanResult::Refused(EconomyError::HasEffects));
}

// ===========================================================================
// Custody locks
// ===========================================================================

#[test]
fn a_live_contracts_deposit_locks_the_object_against_every_other_withdrawal() {
    let mut s = snap();
    s.put_vault(vault(1, 80));
    s.put_asset(asset(60, who(1)));
    let mut c = contract_shell();
    c.consent.conditions = vec![TypedCondition::DepositInto {
        object: obj(60),
        vault: VaultId(1),
    }];
    c.satisfied = vec![None];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.state = ContractState::Active;
    s.put_contract(c.clone());
    assert_eq!(s.contract_locks(), BTreeSet::from([(obj(60), VaultId(1))]));

    let mut k = Seams::open();
    k.custody = Custody::holding(&[60]);
    let mut p = plan("w", who(1), Vec::new());
    p.custody_ops = vec![CustodyOp::Withdraw {
        object: obj(60),
        vault: VaultId(1),
        to: obj(4),
    }];
    assert_eq!(
        refuse_with(&s, &p, &k),
        EconomyError::CustodyLocked(obj(60))
    );

    // The cause ALONE unlocks nothing: `post` is public, so a caller could set it.
    p.cause = EconomicCause::Internal {
        kind: "contract".into(),
        id: c.id.0,
    };
    assert_eq!(
        refuse_with(&s, &p, &k),
        EconomyError::CustodyLocked(obj(60))
    );

    // E4a verify fold, finding 2: NEITHER does the contract's own deterministic
    // key. `contract/{id}/settle` and `contract/{id}/refund` are derived from an id
    // anyone who can see the contract can read, so the pair of them was a lock whose
    // key was printed on the door. There is now no exception at all.
    p.idempotency = settle_key(c.id);
    assert_eq!(
        refuse_with(&s, &p, &k),
        EconomyError::CustodyLocked(obj(60))
    );
    p.idempotency = mush_economy::refund_key(c.id);
    assert_eq!(
        refuse_with(&s, &p, &k),
        EconomyError::CustodyLocked(obj(60))
    );
    p.idempotency = settle_key(ContractId(2));
    assert_eq!(
        refuse_with(&s, &p, &k),
        EconomyError::CustodyLocked(obj(60))
    );

    // The lock lifts on the STATE change, and nothing else: a contract that has left
    // `Active`/`ReadyToSettle` holds nothing, and E4b2's refund then returns the
    // object to its depositor in the same step, under `ContractAuth`.
    s.put_contract(EconomicContract {
        state: ContractState::Cancelled,
        ..c
    });
    assert!(s.contract_locks().is_empty());
    p.idempotency = key("after-cancel");
    assert!(validate(&p, &s, &k.env(&s)).is_post());
}

// ===========================================================================
// Clocks and schedules
// ===========================================================================

#[test]
fn due_periods_counts_a_downtime_gap_and_never_a_backwards_clock() {
    let s = EconomicSchedule::every(1_000, 100);
    assert_eq!(
        due_periods(&s, 900),
        0,
        "a rewound clock yields zero, not a garbage count"
    );
    assert_eq!(due_periods(&s, 1_000), 1);
    assert_eq!(
        due_periods(&s, 1_000 + 100 * 9),
        10,
        "a ten-period downtime gap"
    );
}

#[test]
fn an_ic_schedule_and_a_real_schedule_are_read_on_their_own_clocks() {
    let snapshot = snap();
    let clocks = Clocks::new(NOW, IC);
    let real = EconomicSchedule::new(EconomicClock::RealUtc, NOW - 500, Some(100), 8);
    let ic = EconomicSchedule::new(EconomicClock::WorldIc, IC - 500, Some(100), 8);
    assert_eq!(clocks.due_periods(&real, &snapshot), 6);
    assert_eq!(clocks.due_periods(&ic, &snapshot), 6);
    // An IC jump is only visible on the IC clock.
    let jumped = Clocks::new(NOW, IC + 100_000);
    assert_eq!(jumped.due_periods(&real, &snapshot), 6);
    assert_eq!(jumped.due_periods(&ic, &snapshot), 1006);
}

#[test]
fn the_catch_up_cap_is_clamped_at_construction_and_survives_a_forged_record() {
    let s = EconomicSchedule::new(EconomicClock::RealUtc, 0, Some(10), 250);
    assert_eq!(s.maximum_catch_up, MAX_CATCH_UP_CAP);
    // A record that somehow stored a larger cap still cannot sweep past it.
    let forged = EconomicSchedule {
        maximum_catch_up: 200,
        ..s
    };
    let o = Obligation {
        schedule: forged,
        ..obligation()
    };
    assert_eq!(
        plan_sweep(&o, 10_000).attempt.len(),
        usize::from(MAX_CATCH_UP_CAP)
    );
}

#[test]
fn advancing_a_schedule_walks_whole_periods_and_finishes_a_one_shot() {
    let s = EconomicSchedule::every(1_000, 100);
    assert_eq!(advance(&s, 3).next_due, 1_300);
    let one = EconomicSchedule::new(EconomicClock::RealUtc, 1_000, None, 3);
    assert_eq!(advance(&one, 1).next_due, i64::MAX);
}

#[test]
fn a_cadence_advances_under_its_own_cap_and_forgives_the_rest() {
    let c = CadenceClock {
        id: mush_economy::CadenceId(1),
        name: "season".into(),
        source: CadenceSource::WorldIc,
        period: 100,
        catch_up_cap: 3,
        position: 0,
        advanced_at: 0,
        revision: 1,
        generation: mush_economy::CADENCE_GENERATION_INITIAL,
        schema: mush_economy::CADENCE_SCHEMA_LEGACY,
    };
    let (next, forgiven) = advance_cadence(&c, 1_000);
    assert_eq!((next.position, forgiven), (3, 7));
    let (again, forgiven2) = advance_cadence(&next, 1_000);
    assert_eq!(
        (again.position, forgiven2),
        (3, 0),
        "the forgiven periods never come back"
    );
}

// ===========================================================================
// Obligations
// ===========================================================================

fn obligation() -> Obligation {
    Obligation {
        id: mush_economy::ObligationId(1),
        debtor: who(1),
        creditor: who(2),
        debit_account: AccountId(1),
        credit_account: AccountId(2),
        amount: Amount::from_minor(50),
        schedule: EconomicSchedule::every(1_000, 100),
        grace: GracePolicy { periods: 1 },
        notify_on_success: false,
        status: ObligationStatus::Active,
        cause: ObligationCause::Staff {
            reason: "rent".into(),
        },
        minted_by: None,
        periods_settled: 0,
        history: Vec::new(),
        settled_total: 0,
        missed_total: 0,
        forgiven_total: 0,
        suspensions: std::collections::BTreeSet::new(),
        revision: 1,
    }
}

/// `advance_obligation` for a period the caller knows is the next one.
fn advanced(o: &Obligation, period: u32, outcome: SettleOutcome) -> Obligation {
    advance_obligation(o, period, outcome, NOW).expect("in-order outcome")
}

#[test]
fn the_period_key_is_deterministic_so_a_restart_re_derives_it() {
    let k = obligation_key(mush_economy::ObligationId(12), 7);
    assert_eq!(k.namespace.as_str(), "obligation");
    assert_eq!(k.key.as_str(), "12/7");
    assert_eq!(k, obligation_key(mush_economy::ObligationId(12), 7));
}

#[test]
fn plan_sweep_attempts_at_most_the_catch_up_and_forgives_the_older_excess() {
    let o = Obligation {
        schedule: EconomicSchedule::new(EconomicClock::RealUtc, 1_000, Some(100), 3),
        ..obligation()
    };
    // Ten periods due, catch-up 3: settle the three MOST RECENT, forgive seven.
    let intent = plan_sweep(&o, 1_000 + 100 * 9);
    assert_eq!(intent.forgive, 7);
    assert_eq!((intent.forgive_from, intent.forgive_to), (0, 7));
    assert_eq!(intent.attempt, vec![7, 8, 9]);
    // Three due, catch-up 3: nothing is forgiven.
    let small = plan_sweep(&o, 1_000 + 100 * 2);
    assert_eq!(small.forgive, 0);
    assert_eq!(small.attempt, vec![0, 1, 2]);
}

#[test]
fn a_quarantined_obligation_leaves_the_due_pool_entirely() {
    for status in [
        ObligationStatus::Delinquent,
        ObligationStatus::Suspended {
            reason: mush_economy::SuspendReason::Staff { reason: "x".into() },
        },
        ObligationStatus::Satisfied,
        ObligationStatus::Cancelled,
    ] {
        let o = Obligation {
            status: status.clone(),
            ..obligation()
        };
        assert!(
            plan_sweep(&o, 1_000_000).is_empty(),
            "{status:?} must not starve live obligations"
        );
    }
}

#[test]
fn forgiveness_advances_the_schedule_and_records_why() {
    let o = obligation();
    let next = forgive(&o, 0, 7, ForgiveCause::Downtime, NOW);
    assert_eq!(next.periods_settled, 7);
    assert_eq!(next.schedule.next_due, 1_700);
    assert!(matches!(
        next.history.last(),
        Some(ObligationEvent::Forgiven {
            periods: 7,
            from: 0,
            to: 7,
            cause: ForgiveCause::Downtime,
            ..
        })
    ));
    // Nothing was charged: the whole point of D-E4-14.
    assert!(
        !next
            .history
            .iter()
            .any(|e| matches!(e, ObligationEvent::Settled { .. }))
    );
}

#[test]
fn a_settled_period_advances_the_schedule_and_clears_the_grace_run() {
    let o = Obligation {
        status: ObligationStatus::Grace {
            missed: 1,
            deadline: EconomicDeadline::real(1_100),
        },
        ..obligation()
    };
    let next = advanced(&o, 0, SettleOutcome::Posted(TransactionId(5)));
    assert_eq!(next.status, ObligationStatus::Active);
    assert_eq!(next.periods_settled, 1);
    assert_eq!(next.schedule.next_due, 1_100);
}

#[test]
fn a_one_shot_obligation_is_satisfied_once_it_settles() {
    let o = Obligation {
        schedule: EconomicSchedule::new(EconomicClock::RealUtc, 1_000, None, 3),
        ..obligation()
    };
    let next = advanced(&o, 0, SettleOutcome::AlreadyPosted(TransactionId(5)));
    assert_eq!(next.status, ObligationStatus::Satisfied);
}

#[test]
fn a_missed_period_enters_grace_with_its_deadline_on_the_schedules_own_clock() {
    let o = Obligation {
        schedule: EconomicSchedule::new(EconomicClock::WorldIc, IC, Some(100), 3),
        ..obligation()
    };
    let missed = advanced(&o, 0, SettleOutcome::InsufficientAvailable);
    match missed.status {
        ObligationStatus::Grace {
            missed: n,
            deadline,
        } => {
            assert_eq!(n, 1);
            assert_eq!(
                deadline.clock,
                EconomicClock::WorldIc,
                "never wall time on an IC schedule"
            );
            assert_eq!(deadline.at, IC + 100);
        }
        other => panic!("expected Grace, got {other:?}"),
    }
    assert_eq!(
        missed.periods_settled, 1,
        "the period is CLOSED, so the next key differs"
    );
}

#[test]
fn missing_past_the_grace_allowance_goes_delinquent() {
    let mut o = obligation();
    o = advanced(&o, 0, SettleOutcome::Refused);
    assert!(matches!(
        o.status,
        ObligationStatus::Grace { missed: 1, .. }
    ));
    o = advanced(&o, 1, SettleOutcome::Refused);
    assert_eq!(o.status, ObligationStatus::Delinquent);
    // …and a delinquent obligation records a late payment without silently
    // returning to the due pool: E4b1's `reinstate` (or E5's hook) owns that move.
    let paid = advanced(&o, 2, SettleOutcome::Posted(TransactionId(1)));
    assert_eq!(
        paid.status,
        ObligationStatus::Delinquent,
        "the status is a deliberate act"
    );
    assert_eq!(paid.periods_settled, 3, "the period still closed");
    assert_eq!(
        paid.settled_total, 1,
        "and the payment is recorded, not dropped"
    );
    // A MISS on a delinquent obligation changes nothing at all — it was never
    // attempted.
    assert_eq!(advanced(&o, 2, SettleOutcome::Refused), o);
}

#[test]
fn a_duplicate_outcome_for_a_closed_period_changes_nothing() {
    let o = advanced(&obligation(), 0, SettleOutcome::Posted(TransactionId(1)));
    let again = advanced(&o, 0, SettleOutcome::Posted(TransactionId(1)));
    assert_eq!(
        again, o,
        "a re-delivered sweep result must not move a status twice"
    );
}

#[test]
fn the_history_tail_is_bounded() {
    let mut o = obligation();
    for period in 0..(mush_economy::MAX_OBLIGATION_HISTORY as u32 + 10) {
        o = advanced(&o, period, SettleOutcome::Posted(TransactionId(1)));
    }
    assert_eq!(o.history.len(), mush_economy::MAX_OBLIGATION_HISTORY);
}

#[test]
fn forgiveness_is_classified_from_the_economys_own_sweep_stamps() {
    // No sweep ran for hours; the IC clock kept pace with the ratio.
    assert_eq!(
        classify_forgiveness(0, 0, 10_000, 10_000, 1, false),
        ForgiveCause::Downtime
    );
    // The IC clock outran the ratio: an epoch or ratio change.
    assert_eq!(
        classify_forgiveness(0, 0, 10, 100_000, 1, false),
        ForgiveCause::TimeJump
    );
    // Sweeps ran; this obligation simply never got budget.
    assert_eq!(
        classify_forgiveness(0, 0, 30, 30, 1, false),
        ForgiveCause::Overload
    );
    // A REWOUND clock is a jump, never an overload: `Overload` escalates on repeat,
    // so reading an NTP step as a starved post budget would page a human for nothing.
    assert_eq!(
        classify_forgiveness(500, 500, 400, 400, 1, false),
        ForgiveCause::TimeJump
    );
}

// ===========================================================================
// Contracts
// ===========================================================================

fn terms() -> ContractTerms {
    ContractTerms {
        consideration: vec![Consideration {
            from: who(1),
            account: AccountId(1),
            amount: Amount::from_minor(100),
        }],
        deliverables: Vec::new(),
        split: vec![PayoutShare {
            to: AccountId(2),
            share: Share::Permille(500),
        }],
        remainder_to: AccountId(9),
        on_expiry: Expiry::Refund,
        // E4b2's outcome table; E4a's single split is the degenerate empty case.
        outcomes: Vec::new(),
    }
}

fn contract_shell() -> EconomicContract {
    let consent = ConsentSurface {
        managed: None,
        parties: vec![who(1), who(2)],
        terms: terms(),
        conditions: Vec::new(),
        deadline: None,
        visibility: EconomicVisibility::Parties,
        scope: None,
        // E4b2: the named adjudicator and the permitted-evidence list, both inside
        // the consent hash. `None`/empty is E4a's shape.
        adjudicator: None,
        evidence: Vec::new(),
    };
    let hash = consent_hash(&who(1), &consent);
    EconomicContract {
        id: ContractId(1),
        issuer: who(1),
        name: smol_str::SmolStr::new_static("shell"),
        consent,
        consent_hash: hash,
        acceptances: vec![
            Acceptance {
                party: who(1),
                at: NOW,
                consent_hash: hash,
            },
            Acceptance {
                party: who(2),
                at: NOW,
                consent_hash: hash,
            },
        ],
        satisfied: Vec::new(),
        escrow: vec![HoldId(1)],
        state: ContractState::ReadyToSettle,
        settle_attempts: 0,
        created_at: NOW,
        amendments: 0,
        generation_started_at: 0,
        disputed_at: None,
        canon_published: None,
        roster_locked: true,
        deposits: Vec::new(),
        agreements: Vec::new(),
        pending: None,
        roles: mush_economy::ContractRoles::default(),
        revision: 1,
    }
}

/// A snapshot carrying the escrow hold the shell contract names.
fn contract_snap() -> (EconomySnapshot, EconomicContract) {
    let mut s = snap();
    with_hold(
        &mut s,
        Hold {
            purpose: HoldPurpose::Escrow {
                contract: ContractId(1),
            },
            ..hold(1, 1, 100)
        },
    );
    let c = contract_shell();
    s.put_contract(c.clone());
    (s, c)
}

/// [`settlement_plan`] under the open seams — a contract with no conditions
/// re-verifies vacuously, so this is the plain "build me the settlement" call.
fn settle(c: &EconomicContract, s: &EconomySnapshot) -> Result<EconomyPlan, ContractError> {
    let k = Seams::open();
    settlement_plan(c, s, &k.env(s))
}

/// Re-freeze the consent hash and re-sign it for every party — the shape of a fresh
/// Offer, for a test that edits the surface before exercising it.
fn reoffer(c: &mut EconomicContract) {
    let hash = consent_hash(&c.issuer, &c.consent);
    let acceptances = c
        .consent
        .parties
        .iter()
        .map(|p| Acceptance {
            party: p.clone(),
            at: NOW,
            consent_hash: hash,
        })
        .collect();
    c.consent_hash = hash;
    c.acceptances = acceptances;
}

#[test]
fn the_consent_hash_covers_the_whole_surface_and_ignores_party_order() {
    let base = contract_shell().consent;
    let mut reordered = base.clone();
    reordered.parties.reverse();
    assert_eq!(
        consent_hash(&who(1), &base),
        consent_hash(&who(1), &reordered),
        "parties are a SET"
    );

    type Mutation = (&'static str, Box<dyn Fn(&mut ConsentSurface)>);
    let mutate: Vec<Mutation> = vec![
        (
            "party added",
            Box::new(|c: &mut ConsentSurface| c.parties.push(who(3))),
        ),
        (
            "deadline moved",
            Box::new(|c: &mut ConsentSurface| c.deadline = Some(EconomicDeadline::real(1))),
        ),
        (
            "condition added",
            Box::new(|c: &mut ConsentSurface| {
                c.conditions.push(TypedCondition::Attestation {
                    by: AttestorSet::Staff,
                });
            }),
        ),
        (
            "visibility flipped",
            Box::new(|c: &mut ConsentSurface| c.visibility = EconomicVisibility::Public),
        ),
        (
            "scope set",
            Box::new(|c: &mut ConsentSurface| c.scope = Some(mush_org::OrgScopeId(4))),
        ),
        (
            "remainder repointed",
            Box::new(|c: &mut ConsentSurface| c.terms.remainder_to = AccountId(2)),
        ),
        (
            "amount raised",
            Box::new(|c: &mut ConsentSurface| {
                c.terms.consideration[0].amount = Amount::from_minor(101);
            }),
        ),
    ];
    for (what, f) in mutate {
        let mut changed = base.clone();
        f(&mut changed);
        assert_ne!(
            consent_hash(&who(1), &base),
            consent_hash(&who(1), &changed),
            "{what} must change the hash"
        );
    }
}

#[test]
fn any_consent_surface_change_after_offer_voids_every_acceptance() {
    let (s, mut c) = contract_snap();
    assert!(c.fully_accepted());
    assert!(settle(&c, &s).is_ok());
    // The issuer edits the surface after Offer: the frozen hash no longer describes it.
    c.consent.terms.remainder_to = AccountId(2);
    assert!(!c.consent_intact());
    assert!(!c.accepted_by(&who(1)));
    assert_eq!(settle(&c, &s), Err(ContractError::ConsentMismatch));
}

#[test]
fn an_acceptance_bound_to_a_different_hash_does_not_count() {
    let (s, mut c) = contract_snap();
    c.acceptances[1].consent_hash = [0u8; 32];
    assert!(!c.accepted_by(&who(2)));
    assert_eq!(settle(&c, &s), Err(ContractError::NotAccepted));
}

#[test]
fn scenario_17_the_settlement_split_floors_permille_shares_and_pays_the_remainder() {
    let (mut s, mut c) = contract_snap();
    // 100 escrowed, three equal thirds: 33/33/33 and the leftover 1 where declared.
    c.consent.terms.split = vec![
        PayoutShare {
            to: AccountId(2),
            share: Share::Permille(333),
        },
        PayoutShare {
            to: AccountId(3),
            share: Share::Permille(333),
        },
        PayoutShare {
            to: AccountId(4),
            share: Share::Permille(333),
        },
    ];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    for a in [3, 4] {
        s.put_account(account(a, 0));
    }
    c.acceptances = vec![
        Acceptance {
            party: who(1),
            at: NOW,
            consent_hash: c.consent_hash,
        },
        Acceptance {
            party: who(2),
            at: NOW,
            consent_hash: c.consent_hash,
        },
    ];
    let p = settle(&c, &s).expect("settlement plan");
    assert_eq!(p.idempotency, settle_key(ContractId(1)));
    let mut paid: BTreeMap<u64, i64> = BTreeMap::new();
    for l in &p.requested_lines {
        *paid.entry(l.account.0).or_insert(0) += l.delta.minor();
    }
    assert_eq!(
        paid,
        BTreeMap::from([(1, -100), (2, 33), (3, 33), (4, 33), (9, 1)])
    );

    // …and the plan actually posts under the contract's own authority.
    let auth = ContractAuth::new(&c, &s);
    let env = ValidateEnv {
        keys: &mush_economy::MapOnlyKeys,
        auth: &auth,
        custody: &Custody::default(),
        facts: &Facts::default(),
        now: NOW,
        ic_now: IC,
        next_hold_id: s.next_hold_floor(),
        spent: std::collections::BTreeMap::new(),
    };
    let prepared = match validate(&p, &s, &env) {
        PlanResult::Post(prepared) => *prepared,
        other => panic!("expected Post, got {other:?}"),
    };
    let (s2, _) = apply(&s, &prepared, TransactionId(1), 1, NOW);
    assert_eq!(
        s2.account(AccountId(1)).unwrap().posted_balance,
        Amount::from_minor(900)
    );
    assert_eq!(s2.account(AccountId(1)).unwrap().held_amount, Amount::ZERO);
    assert_eq!(
        s2.account(AccountId(9)).unwrap().posted_balance,
        Amount::from_minor(1)
    );
}

#[test]
fn contract_auth_authorizes_exactly_the_surface_and_refuses_an_extra_line() {
    let (s, c) = contract_snap();
    let auth = ContractAuth::new(&c, &s);
    let acct1 = s.account(AccountId(1)).unwrap();
    let acct2 = s.account(AccountId(2)).unwrap();
    let acct9 = s.account(AccountId(9)).unwrap();
    // E4a verify fold, finding 1: a contract grants NO bare debit authority. Its
    // power to take money is its power to capture its own escrow, which the engine
    // excuses from the debit question on its own — so anything still asked here is a
    // line beyond the escrow.
    assert!(!auth.authorize(&who(1), acct1, Amount::from_minor(-100)));
    assert!(!auth.authorize(&who(1), acct1, Amount::from_minor(-1)));
    assert!(
        auth.authorize(&who(1), acct2, Amount::from_minor(50)),
        "a declared payee"
    );
    assert!(
        auth.authorize(&who(1), acct9, Amount::from_minor(50)),
        "the remainder account"
    );
    // An account nobody named cannot be credited by this contract.
    let stranger = account(77, 0);
    assert!(!auth.authorize(&who(1), &stranger, Amount::from_minor(1)));
    assert!(!auth.authorize(&who(1), &stranger, Amount::from_minor(-1)));

    // A whole extra line pair through the real engine.
    let mut p = settle(&c, &s).unwrap();
    p.requested_lines.push(line(1, -10));
    p.requested_lines.push(line(2, 10));
    let env = ValidateEnv {
        keys: &mush_economy::MapOnlyKeys,
        auth: &auth,
        custody: &Custody::default(),
        facts: &Facts::default(),
        now: NOW,
        ic_now: IC,
        next_hold_id: s.next_hold_floor(),
        spent: std::collections::BTreeMap::new(),
    };
    assert_eq!(
        validate(&p, &s, &env),
        PlanResult::Refused(EconomyError::Unauthorized)
    );
}

#[test]
fn contract_auth_refuses_an_over_amount_capture_and_a_foreign_hold() {
    let (s, c) = contract_snap();
    let auth = ContractAuth::new(&c, &s);
    let h = s.hold(HoldId(1)).unwrap();
    let ok = HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::from_minor(100),
    };
    let over = HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::from_minor(101),
    };
    assert!(auth.authorize_hold(&who(1), h, &ok));
    assert!(!auth.authorize_hold(&who(1), h, &over));
    assert!(!auth.authorize_hold(&who(1), h, &HoldOp::Expire { hold: HoldId(1) }));
    // A hold this contract does not escrow is not its business.
    let foreign = hold(2, 1, 100);
    assert!(!auth.authorize_hold(&who(1), &foreign, &HoldOp::Release { hold: HoldId(2) }));
}

#[test]
fn contract_auth_moves_title_only_where_the_surface_says_and_only_between_acceptors() {
    let (mut s, mut c) = contract_snap();
    s.put_asset(asset(60, who(1)));
    s.put_asset(asset(61, who(1)));
    c.consent.terms.deliverables = vec![Deliverable::TitleTransfer {
        object: obj(60),
        from: who(1),
        to: who(2),
    }];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.acceptances = vec![
        Acceptance {
            party: who(1),
            at: NOW,
            consent_hash: c.consent_hash,
        },
        Acceptance {
            party: who(2),
            at: NOW,
            consent_hash: c.consent_hash,
        },
    ];
    let auth = ContractAuth::new(&c, &s);
    let listed = AssetOp::TransferTitle {
        object: obj(60),
        to: who(2),
        to_scope: None,
    };
    let unlisted = AssetOp::TransferTitle {
        object: obj(61),
        to: who(2),
        to_scope: None,
    };
    let elsewhere = AssetOp::TransferTitle {
        object: obj(60),
        to: who(3),
        to_scope: None,
    };
    assert!(auth.authorize_asset(&who(1), s.asset(obj(60)), &listed));
    assert!(
        !auth.authorize_asset(&who(1), s.asset(obj(61)), &unlisted),
        "an unlisted object"
    );
    assert!(
        !auth.authorize_asset(&who(1), s.asset(obj(60)), &elsewhere),
        "an unlisted destination"
    );
    assert!(auth.authorize_title_receipt(&who(1), &who(2), None));
    assert!(
        !auth.authorize_title_receipt(&who(1), &who(3), None),
        "a non-party receives nothing"
    );

    // D-E3-12: drop the RECEIVING party's acceptance and the transfer dies.
    let mut half = c.clone();
    half.acceptances.retain(|a| a.party != who(2));
    let half_auth = ContractAuth::new(&half, &s);
    assert!(!half_auth.authorize_asset(&who(1), s.asset(obj(60)), &listed));
}

#[test]
fn contract_auth_places_and_releases_only_its_own_collateral() {
    let (mut s, mut c) = contract_snap();
    s.put_asset(asset(60, who(2)));
    s.put_asset(asset(61, who(2)));
    c.consent.terms.deliverables = vec![Deliverable::Collateral {
        object: obj(60),
        from: who(2),
    }];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.acceptances = vec![
        Acceptance {
            party: who(1),
            at: NOW,
            consent_hash: c.consent_hash,
        },
        Acceptance {
            party: who(2),
            at: NOW,
            consent_hash: c.consent_hash,
        },
    ];
    let auth = ContractAuth::new(&c, &s);
    let mine = Encumbrance::Collateral {
        contract: ContractId(1),
    };
    let theirs = Encumbrance::Collateral {
        contract: ContractId(2),
    };
    let lien = Encumbrance::Lien {
        creditor: who(1),
        amount: Amount::from_minor(1),
        instrument: iid("gold"),
    };
    let pledge = AssetOp::Encumber {
        object: obj(60),
        encumbrance: mine.clone(),
    };
    assert!(auth.authorize_asset(&who(1), s.asset(obj(60)), &pledge));
    assert!(auth.authorize_asset(
        &who(1),
        s.asset(obj(60)),
        &AssetOp::Release {
            object: obj(60),
            encumbrance: mine.clone(),
        }
    ));
    // Another contract's claim, an unpledged object, and a bare lien are all refused —
    // a contract is not a licence to encumber whatever it can name (D-E4-8).
    assert!(!auth.authorize_asset(
        &who(1),
        s.asset(obj(60)),
        &AssetOp::Encumber {
            object: obj(60),
            encumbrance: theirs,
        }
    ));
    assert!(!auth.authorize_asset(
        &who(1),
        s.asset(obj(61)),
        &AssetOp::Encumber {
            object: obj(61),
            encumbrance: mine,
        }
    ));
    assert!(!auth.authorize_asset(
        &who(1),
        s.asset(obj(60)),
        &AssetOp::Encumber {
            object: obj(60),
            encumbrance: lien,
        }
    ));
}

#[test]
fn the_escrow_ceiling_counts_only_this_contracts_own_holds() {
    let (mut s, c) = contract_snap();
    // A second, unrelated hold on the same account must not raise what THIS contract
    // may take — escrow is per-contract, not per-account.
    with_hold(
        &mut s,
        Hold {
            purpose: HoldPurpose::Escrow {
                contract: ContractId(2),
            },
            ..hold(2, 1, 500)
        },
    );
    let auth = ContractAuth::new(&c, &s);
    assert_eq!(
        c.escrowed_by_account(&s),
        BTreeMap::from([(AccountId(1), Amount::from_minor(100))]),
        "the other contract's 500 is not this contract's money"
    );
    let foreign = s.hold(HoldId(2)).unwrap();
    assert!(!auth.authorize_hold(&who(1), foreign, &HoldOp::Release { hold: HoldId(2) }));
    // …and a released escrow hold stops counting at all.
    let mut broken = s.clone();
    broken.put_hold(Hold {
        status: HoldStatus::Released,
        purpose: HoldPurpose::Escrow {
            contract: ContractId(1),
        },
        ..hold(1, 1, 100)
    });
    let after = ContractAuth::new(&c, &broken);
    assert!(c.escrowed_by_account(&broken).is_empty());
    let dead = broken.hold(HoldId(1)).unwrap();
    assert!(!after.authorize_hold(&who(1), dead, &HoldOp::Release { hold: HoldId(1) }));
}

#[test]
fn check_offerable_refuses_an_infeasible_split_and_an_oversized_contract() {
    let (s, base) = contract_snap();
    assert!(check_offerable(&base, &s).is_ok());

    let over_exact = with_split(
        &base,
        vec![PayoutShare {
            to: AccountId(2),
            share: Share::Exact(Amount::from_minor(500)),
        }],
    );
    assert_eq!(
        check_offerable(&over_exact, &s),
        Err(ContractError::SplitInfeasible)
    );

    let over_permille = with_split(
        &base,
        vec![
            PayoutShare {
                to: AccountId(2),
                share: Share::Permille(600),
            },
            PayoutShare {
                to: AccountId(9),
                share: Share::Permille(600),
            },
        ],
    );
    assert_eq!(
        check_offerable(&over_permille, &s),
        Err(ContractError::SplitInfeasible)
    );

    let mut too_many = base.clone();
    too_many.consent.parties = (0..20).map(|n| who(100 + n)).collect();
    assert!(matches!(
        check_offerable(&too_many, &s),
        Err(ContractError::TooLarge(_))
    ));

    let mut two_deposits = base.clone();
    two_deposits.consent.terms.deliverables = vec![
        Deliverable::DepositInto {
            object: obj(60),
            vault: VaultId(1),
        },
        Deliverable::DepositInto {
            object: obj(61),
            vault: VaultId(1),
        },
    ];
    // E3's one-custody-op-per-plan module cap refuses the second deposit at Offer,
    // rather than at a half-applied settlement — and it is a **structural** refusal
    // since the E4b2 verify fold split `check_structure` out of `check_offerable`, so
    // it fires whether or not the vault exists (which is what lets boot re-run it).
    assert!(matches!(
        check_offerable(&two_deposits, &s),
        Err(ContractError::Infeasible(_))
    ));
    let mut with_vault = s.clone();
    with_vault.put_vault(vault(1, 80));
    assert!(matches!(
        check_offerable(&two_deposits, &with_vault),
        Err(ContractError::Infeasible(_))
    ));
    let mut one_deposit = two_deposits.clone();
    one_deposit.consent.terms.deliverables.truncate(1);
    // With one deposit, the world half is what answers: no vault is `NoSuchVault`.
    assert!(matches!(
        check_offerable(&one_deposit, &s),
        Err(ContractError::NoSuchVault(_))
    ));
    assert!(check_offerable(&one_deposit, &with_vault).is_ok());

    let mut too_many_deliverables = base.clone();
    too_many_deliverables.consent.terms.deliverables = (0..20)
        .map(|i| Deliverable::Collateral {
            object: obj(200 + i),
            from: who(1),
        })
        .collect();
    assert!(matches!(
        check_offerable(&too_many_deliverables, &s),
        Err(ContractError::TooLarge(_))
    ));

    let mut unknown_account = base.clone();
    unknown_account.consent.terms.remainder_to = AccountId(404);
    assert_eq!(
        check_offerable(&unknown_account, &s),
        Err(ContractError::NoSuchAccount(AccountId(404)))
    );

    let mut self_anchor = base;
    self_anchor.consent.conditions = vec![TypedCondition::After {
        condition: 0,
        delay: 1,
        clock: EconomicClock::RealUtc,
    }];
    self_anchor.satisfied = vec![None];
    assert_eq!(
        check_offerable(&self_anchor, &s),
        Err(ContractError::BadConditionRef(0))
    );
}

fn with_split(c: &EconomicContract, split: Vec<PayoutShare>) -> EconomicContract {
    let mut out = c.clone();
    out.consent.terms.split = split;
    out.consent_hash = consent_hash(&out.issuer, &out.consent);
    out.acceptances = out
        .consent
        .parties
        .iter()
        .map(|p| Acceptance {
            party: p.clone(),
            at: NOW,
            consent_hash: out.consent_hash,
        })
        .collect();
    out
}

#[test]
fn settlement_refuses_a_broken_escrow_and_a_wrong_state() {
    let (mut s, mut c) = contract_snap();
    c.state = ContractState::Offered;
    assert_eq!(settle(&c, &s), Err(ContractError::State(ContractId(1))));
    c.state = ContractState::ReadyToSettle;
    s.put_hold(Hold {
        status: HoldStatus::Released,
        ..hold(1, 1, 100)
    });
    assert_eq!(settle(&c, &s), Err(ContractError::HoldNotHeld(HoldId(1))));
    c.escrow = vec![HoldId(42)];
    assert_eq!(settle(&c, &s), Err(ContractError::NoSuchHold(HoldId(42))));
}

#[test]
fn the_refund_plan_releases_every_escrow_hold_and_moves_no_money() {
    let (mut s, mut c) = contract_snap();
    // A refund follows the STATE change (finding 6): the contract is cancelled
    // first, which also lifts any custody lock, and only then is the escrow freed.
    c.state = ContractState::Cancelled;
    s.put_contract(c.clone());
    let p = refund_plan(&c, &s).expect("refund plan");
    assert!(p.requested_lines.is_empty(), "a refund is effect-only");
    assert_eq!(p.hold_ops, vec![HoldOp::Release { hold: HoldId(1) }]);
    // E4b2 narrows `ContractAuth`: a SETTLEMENT authority may capture and may not
    // release, and an unwind is the other way round. A settlement that could release
    // its own escrow could break the agreement it is performing.
    let settling = ContractAuth::new(&c, &s);
    let hold = s.hold(HoldId(1)).expect("the escrow hold");
    assert!(
        !settling.authorize_hold(&who(1), hold, &HoldOp::Release { hold: HoldId(1) }),
        "a settlement authority released escrow"
    );
    let auth = ContractAuth::unwinding(&c, &s);
    assert!(
        !auth.authorize_hold(
            &who(1),
            hold,
            &HoldOp::Capture {
                hold: HoldId(1),
                amount: Amount::from_minor(1),
            }
        ),
        "an unwind authority captured escrow"
    );
    let env = ValidateEnv {
        keys: &mush_economy::MapOnlyKeys,
        auth: &auth,
        custody: &Custody::default(),
        facts: &Facts::default(),
        now: NOW,
        ic_now: IC,
        next_hold_id: s.next_hold_floor(),
        spent: std::collections::BTreeMap::new(),
    };
    let prepared = match validate(&p, &s, &env) {
        PlanResult::Post(prepared) => *prepared,
        other => panic!("expected Post, got {other:?}"),
    };
    let (s2, _) = apply(&s, &prepared, TransactionId(1), 1, NOW);
    assert_eq!(
        s2.account(AccountId(1)).unwrap().posted_balance,
        Amount::from_minor(1000)
    );
    assert_eq!(s2.account(AccountId(1)).unwrap().held_amount, Amount::ZERO);
}

#[test]
fn reverify_catches_a_withdrawn_deposit_and_a_de_qualified_attestor() {
    let (mut s, mut c) = contract_snap();
    s.put_vault(vault(1, 80));
    s.put_asset(asset(60, who(2)));
    let cap = CapabilityId::new("asset.assign").unwrap();
    c.consent.conditions = vec![
        TypedCondition::DepositInto {
            object: obj(60),
            vault: VaultId(1),
        },
        TypedCondition::TitleHeld {
            object: obj(60),
            by: who(2),
        },
        TypedCondition::Attestation {
            by: AttestorSet::Capability {
                scope: None,
                capability: cap,
            },
        },
    ];
    c.satisfied = vec![
        Some(att(AttestorRef::Clock)),
        Some(att(AttestorRef::Clock)),
        Some(att(AttestorRef::Principal(who(5)))),
    ];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);

    let mut k = Seams::open();
    k.custody = Custody::holding(&[60]);
    assert!(reverify(&c, &s, &k.env(&s)).is_ok());

    // The seller withdrew the goods after attesting.
    let mut gone = Seams::open();
    assert_eq!(
        reverify(&c, &s, &gone.env(&s)),
        Err(ContractError::ConditionFailed(0))
    );
    gone.custody = Custody::holding(&[60]);

    // Title moved away after attestation.
    let mut moved = s.clone();
    moved.put_asset(asset(60, who(3)));
    assert_eq!(
        reverify(&c, &moved, &k.env(&moved)),
        Err(ContractError::ConditionFailed(1))
    );

    // The attestor lost the capability between attesting and settlement (A5).
    let mut lost = Seams::open();
    lost.custody = Custody::holding(&[60]);
    lost.auth.attestor = false;
    assert_eq!(
        reverify(&c, &s, &lost.env(&s)),
        Err(ContractError::ConditionFailed(2))
    );
}

fn att(by: AttestorRef) -> Attestation {
    Attestation {
        by,
        at: NOW,
        clock: EconomicClock::RealUtc,
        source: "test".into(),
    }
}

#[test]
fn an_unanswerable_fact_is_unsatisfied_and_never_a_pass() {
    let (s, mut c) = contract_snap();
    let fact = mush_economy::ExternalRef {
        provider: "jobs".into(),
        kind: "resolved".into(),
        id: "7".into(),
        revision: None,
    };
    c.consent.conditions = vec![TypedCondition::Fact { fact }];
    c.satisfied = vec![Some(att(AttestorRef::Clock))];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);

    let mut unknown = Seams::open();
    unknown.facts.answer = None;
    assert_eq!(
        reverify(&c, &s, &unknown.env(&s)),
        Err(ContractError::ConditionFailed(0))
    );
    let mut no = Seams::open();
    no.facts.answer = Some(false);
    assert_eq!(
        reverify(&c, &s, &no.env(&s)),
        Err(ContractError::ConditionFailed(0))
    );
    let mut yes = Seams::open();
    yes.facts.answer = Some(true);
    assert!(reverify(&c, &s, &yes.env(&s)).is_ok());
}

#[test]
fn an_after_condition_becomes_true_only_delay_after_its_anchor_and_only_on_one_clock() {
    let (s, mut c) = contract_snap();
    c.consent.conditions = vec![
        TypedCondition::Attestation {
            by: AttestorSet::Party(who(2)),
        },
        TypedCondition::After {
            condition: 0,
            delay: 100,
            clock: EconomicClock::RealUtc,
        },
    ];
    c.satisfied = vec![
        Some(Attestation {
            by: AttestorRef::Principal(who(2)),
            at: 1_000,
            clock: EconomicClock::RealUtc,
            source: "attest".into(),
        }),
        Some(att(AttestorRef::Clock)),
    ];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);

    let mut early = Seams::open();
    early.now = 1_099;
    assert_eq!(
        reverify(&c, &s, &early.env(&s)),
        Err(ContractError::ConditionFailed(1))
    );
    let mut ripe = Seams::open();
    ripe.now = 1_100;
    assert!(reverify(&c, &s, &ripe.env(&s)).is_ok());

    // An anchor stamped on the IC clock can never satisfy a real-time delay.
    let mut mixed = c;
    mixed.satisfied[0] = Some(Attestation {
        by: AttestorRef::Principal(who(2)),
        at: 1_000,
        clock: EconomicClock::WorldIc,
        source: "attest".into(),
    });
    mixed.consent_hash = consent_hash(&mixed.issuer, &mixed.consent);
    assert_eq!(
        reverify(&mixed, &s, &ripe.env(&s)),
        Err(ContractError::ConditionFailed(1))
    );
}

#[test]
fn a_party_attestation_stands_but_only_from_that_party() {
    let (s, mut c) = contract_snap();
    c.consent.conditions = vec![TypedCondition::Attestation {
        by: AttestorSet::Party(who(2)),
    }];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.satisfied = vec![Some(att(AttestorRef::Principal(who(2))))];
    let k = Seams::open();
    assert!(reverify(&c, &s, &k.env(&s)).is_ok());
    c.satisfied = vec![Some(att(AttestorRef::Principal(who(3))))];
    assert_eq!(
        reverify(&c, &s, &k.env(&s)),
        Err(ContractError::ConditionFailed(0))
    );
    c.satisfied = vec![None];
    assert_eq!(
        reverify(&c, &s, &k.env(&s)),
        Err(ContractError::ConditionFailed(0))
    );
}

// ===========================================================================
// plan_hash v3
// ===========================================================================

#[test]
fn v3_leaves_an_e3_shaped_plan_byte_identical() {
    // The lib's own `PIN_DIGEST`/`PIN_DIGEST_V3` unit tests pin the exact digests;
    // this proves the property the pins protect from the caller's side.
    let mut base = plan("pin", who(7), vec![line(1, -250), line(2, 250)]);
    base.asset_ops = vec![AssetOp::Unassign { object: obj(60) }];
    let mut with_empty = base.clone();
    with_empty.hold_ops = Vec::new();
    with_empty.expected_hold_revisions = Vec::new();
    assert_eq!(
        plan_hash(&base),
        plan_hash(&with_empty),
        "an empty hold_ops vector is ABSENT from the canonical bytes, not an empty array"
    );
    let mut with_hold = base.clone();
    with_hold.hold_ops = vec![place(1, 5)];
    assert_ne!(
        plan_hash(&base),
        plan_hash(&with_hold),
        "a hold op IS part of the identity"
    );
}

#[test]
fn hold_op_order_never_changes_the_hash_or_the_applied_effects() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 100));
    with_hold(&mut s, hold(2, 1, 100));
    let ops = vec![
        HoldOp::Release { hold: HoldId(2) },
        place(1, 30),
        HoldOp::Release { hold: HoldId(1) },
    ];
    let mut reversed = ops.clone();
    reversed.reverse();
    let a = hold_plan("order", ops);
    let b = hold_plan("order", reversed);
    assert_eq!(plan_hash(&a), plan_hash(&b));
    let pa = prepare(&s, &a, &Seams::open());
    let pb = prepare(&s, &b, &Seams::open());
    assert_eq!(
        pa.hold_effects, pb.hold_effects,
        "applied order == hashed order"
    );
    // The order is by SUBJECT id first (account 1 for the placement, holds 1 and 2
    // for the releases), so the placement leads here — not because a `Place` sorts
    // first in general, but because the key is total and both plans compute it the
    // same way, which is all a stable minted id needs.
    assert_eq!(pa.hold_effects[0].kind, HoldEffectKind::Placed);
}

#[test]
fn the_expectations_stay_outside_the_hash() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 100));
    let base = hold_plan("exp", vec![HoldOp::Release { hold: HoldId(1) }]);
    let mut guarded = base.clone();
    guarded.expected_hold_revisions = vec![HoldRevision {
        hold: HoldId(1),
        revision: 1,
    }];
    assert_eq!(
        plan_hash(&base),
        plan_hash(&guarded),
        "an approval authorizes an intent, not a state"
    );
}

// ===========================================================================
// Fuzz
// ===========================================================================

#[test]
fn fuzz_random_hold_and_asset_sequences_never_panic_and_never_post_an_empty_transaction() {
    let mut s = snap();
    // Two funded accounts moving value in both directions, so the run does not
    // simply drain one of them and then refuse every placement for the rest of it.
    s.put_account(account(1, 500));
    s.put_account(account(2, 500));
    s.put_vault(vault(1, 80));
    for i in 0..3u32 {
        s.put_asset(asset(60 + i, who(1)));
    }
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut k = Seams::open();
    k.custody = Custody::holding(&[60, 61, 62]);
    let mut txid = 1u64;
    for step in 0..1500u64 {
        let object = obj(60 + (next() % 3) as u32);
        let mut p = plan(&format!("f{step}"), who(1), Vec::new());
        p.cause = if next() % 5 == 0 {
            EconomicCause::Internal {
                kind: "hold.expire".into(),
                id: step,
            }
        } else {
            EconomicCause::Staff {
                reason: "fuzz".into(),
            }
        };
        if next() % 2 == 0 {
            let amt = (next() % 200) as i64 + 1;
            let (from, to) = if next() % 2 == 0 { (1, 2) } else { (2, 1) };
            p.requested_lines = vec![line(from, -amt), line(to, amt)];
        }
        // A live hold to aim the non-placing ops at (or a wild id, which must refuse).
        let live: Vec<HoldId> = s
            .holds()
            .filter(|h| h.status.is_held())
            .map(|h| h.id)
            .collect();
        let target = live
            .get((next() % (live.len() as u64 + 1).max(1)) as usize % live.len().max(1))
            .copied()
            .unwrap_or(HoldId(999));
        match next() % 7 {
            0 => p.hold_ops = vec![place(1 + next() % 2, (next() % 200) as i64 + 1)],
            1 => {
                p.hold_ops = vec![HoldOp::Capture {
                    hold: target,
                    amount: Amount::from_minor((next() % 300) as i64 + 1),
                }];
            }
            2 => p.hold_ops = vec![HoldOp::Release { hold: target }],
            3 => p.hold_ops = vec![HoldOp::Expire { hold: target }],
            4 => {
                p.asset_ops = vec![AssetOp::TransferTitle {
                    object,
                    to: who(2),
                    to_scope: None,
                }]
            }
            5 => {
                p.asset_ops = vec![AssetOp::Encumber {
                    object,
                    encumbrance: Encumbrance::Collateral {
                        contract: ContractId(1),
                    },
                }];
            }
            _ => {
                p.custody_ops = vec![CustodyOp::Deposit {
                    object,
                    vault: VaultId(1),
                }]
            }
        }
        match validate(&p, &s, &k.env(&s)) {
            PlanResult::Post(prepared) => {
                assert!(
                    prepared.lines.len() >= 2
                        || !prepared.asset_effects.is_empty()
                        || !prepared.custody_effects.is_empty()
                        || !prepared.hold_effects.is_empty(),
                    "invariant 10: an accepted transaction is never empty"
                );
                let (s2, _) = apply(&s, &prepared, TransactionId(txid), txid, NOW);
                s = s2;
                txid += 1;
            }
            PlanResult::Refused(_)
            | PlanResult::AlreadyPosted(_)
            | PlanResult::NeedsApproval(_) => {}
        }
        // Conservation, and the projection the whole cut exists to keep honest.
        let sum: i128 = s.accounts().map(|a| a.posted_balance.widen()).sum();
        assert_eq!(sum, 1000, "gold conservation broke at step {step}");
        for a in s.accounts() {
            let derived: i128 = s
                .holds_of(a.id)
                .filter(|h| h.status.is_held())
                .map(|h| h.amount.widen())
                .sum();
            assert_eq!(
                a.held_amount.widen(),
                derived,
                "held_amount drifted at step {step}"
            );
            assert!(
                !a.available().expect("available fits").is_negative(),
                "available went negative"
            );
        }
    }
    // The run is only worth anything if it actually reached the hold paths.
    let placed = s.holds().count();
    let terminal = s.holds().filter(|h| !h.status.is_held()).count();
    assert!(placed > 20, "the fuzz never placed a hold ({placed})");
    assert!(
        terminal > 5,
        "the fuzz never captured/released/expired one ({terminal})"
    );
}

// ===========================================================================
// ---- E4a verify fold ----
//
// One test per confirmed finding of the end-of-build adversarial verify. Each
// names the property the old code got wrong, so a future refactor that
// re-introduces it fails here rather than in a game.
// ===========================================================================

/// An [`AuthEval`] that answers the hold seam yes, allows every credit, and allows a
/// debit only down to `-cap`. `DebitCap(0)` is "no debit authority whatsoever".
struct DebitCap(i64);
impl AuthEval for DebitCap {
    fn authorize(&self, _: &PrincipalId, _: &Account, delta: Amount) -> bool {
        !delta.is_negative() || delta.minor() >= -self.0
    }
    fn authorize_hold(&self, _: &PrincipalId, _: &Hold, _: &HoldOp) -> bool {
        true
    }
}

fn validate_with(p: &EconomyPlan, s: &EconomySnapshot, auth: &dyn AuthEval) -> PlanResult {
    let custody = Custody::default();
    let facts = Facts::default();
    validate(
        p,
        s,
        &ValidateEnv {
            keys: &mush_economy::MapOnlyKeys,
            auth,
            custody: &custody,
            facts: &facts,
            now: NOW,
            ic_now: IC,
            next_hold_id: s.next_hold_floor(),
            spent: std::collections::BTreeMap::new(),
        },
    )
}

#[test]
fn vf_1_a_capture_must_be_backed_by_a_debit_on_its_own_account() {
    let mut s = snap();
    s.put_account(account(2, 500));
    with_hold(&mut s, hold(1, 1, 100));
    let capture = HoldOp::Capture {
        hold: HoldId(1),
        amount: Amount::from_minor(100),
    };

    // An EFFECT-ONLY capture destroys a reservation and moves nothing: the payer's
    // funds are freed and the beneficiary gets nothing.
    assert_eq!(
        refuse(&s, &hold_plan("eo", vec![capture.clone()])),
        EconomyError::CaptureUnbacked(HoldId(1))
    );

    // …and a capture on one account paired with a debit on ANOTHER spends the first
    // account's escrow to authorize the second account's money.
    let mut crossed = plan("xy", who(1), vec![line(2, -100), line(9, 100)]);
    crossed.hold_ops = vec![capture.clone()];
    assert_eq!(
        refuse(&s, &crossed),
        EconomyError::CaptureUnbacked(HoldId(1))
    );

    // A capture with a covering debit posts under the hold seam ALONE: the
    // placement already proved debit authority, and that is what a hold is for.
    let mut covered = plan("cov", who(1), vec![line(1, -100), line(2, 100)]);
    covered.hold_ops = vec![capture.clone()];
    assert!(validate_with(&covered, &s, &DebitCap(0)).is_post());

    // A LARGER debit needs live authority for the excess — and only for the excess.
    let mut larger = plan("more", who(1), vec![line(1, -150), line(2, 150)]);
    larger.hold_ops = vec![capture];
    assert!(
        validate_with(&larger, &s, &DebitCap(50)).is_post(),
        "50 uncovered, 50 permitted"
    );
    assert_eq!(
        validate_with(&larger, &s, &DebitCap(49)),
        PlanResult::Refused(EconomyError::Unauthorized),
        "the excess is asked about, and it is asked honestly"
    );
    assert_eq!(
        validate_with(&larger, &s, &DebitCap(0)),
        PlanResult::Refused(EconomyError::Unauthorized)
    );
}

#[test]
fn vf_2_a_custody_lock_has_no_exception_at_all() {
    let mut s = snap();
    s.put_vault(vault(1, 80));
    s.put_asset(asset(60, who(1)));
    let mut c = contract_shell();
    c.consent.conditions = vec![TypedCondition::DepositInto {
        object: obj(60),
        vault: VaultId(1),
    }];
    c.satisfied = vec![None];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    let mut k = Seams::open();
    k.custody = Custody::holding(&[60]);
    let withdraw = || CustodyOp::Withdraw {
        object: obj(60),
        vault: VaultId(1),
        to: obj(4),
    };

    // Neither the contract's cause nor either of its two deterministic keys opens
    // the lock: both are derived from an id anyone who can see the contract can
    // read, so the pair of them was a lock with its key printed on the door.
    for state in [ContractState::Active, ContractState::ReadyToSettle] {
        s.put_contract(EconomicContract { state, ..c.clone() });
        for id in [
            settle_key(c.id),
            mush_economy::refund_key(c.id),
            key("plain"),
        ] {
            let mut p = plan("w", who(1), Vec::new());
            p.idempotency = id;
            p.cause = EconomicCause::Internal {
                kind: "contract".into(),
                id: c.id.0,
            };
            p.custody_ops = vec![withdraw()];
            assert_eq!(
                refuse_with(&s, &p, &k),
                EconomyError::CustodyLocked(obj(60)),
                "{state:?}"
            );
        }
    }

    // The lock lifts on the STATE change and on nothing else. E4b2's refund returns
    // the object to its depositor with a `ContractAuth`-authorized `Withdraw` after
    // that change, in the same step.
    for state in [
        ContractState::Cancelled,
        ContractState::Expired,
        ContractState::Settled {
            transaction: TransactionId(1),
        },
    ] {
        s.put_contract(EconomicContract { state, ..c.clone() });
        let mut p = plan("w2", who(1), Vec::new());
        p.custody_ops = vec![withdraw()];
        assert!(validate(&p, &s, &k.env(&s)).is_post(), "{state:?}");
    }
}

#[test]
fn vf_3_a_hold_merely_listed_in_escrow_is_not_escrow() {
    let (mut s, mut c) = contract_snap();
    // An outsider's own `Manual` hold, listed in `escrow` by whoever can edit the
    // record. It is not this contract's escrow and must not become it.
    s.put_account(account(3, 500));
    with_hold(
        &mut s,
        Hold {
            placed_by: who(3),
            ..hold(7, 3, 500)
        },
    );
    c.escrow = vec![HoldId(1), HoldId(7)];
    s.put_contract(c.clone());

    // 1. The escrowed total ignores it…
    assert_eq!(
        c.escrowed_by_account(&s),
        BTreeMap::from([(AccountId(1), Amount::from_minor(100))])
    );
    // 2. …so does the contract's authority…
    let auth = ContractAuth::new(&c, &s);
    let outsider = s.hold(HoldId(7)).unwrap();
    assert!(!auth.authorize_hold(&who(1), outsider, &HoldOp::Release { hold: HoldId(7) }));
    assert!(!auth.authorize_hold(
        &who(1),
        outsider,
        &HoldOp::Capture {
            hold: HoldId(7),
            amount: Amount::from_minor(1),
        }
    ));
    // 3. …so does the settlement, which captures and pays out only the real escrow…
    let p = settle(&c, &s).expect("the real escrow is complete");
    assert_eq!(
        p.hold_ops,
        vec![HoldOp::Capture {
            hold: HoldId(1),
            amount: Amount::from_minor(100)
        }]
    );
    let debited: i64 = p
        .requested_lines
        .iter()
        .map(|l| l.delta.minor())
        .filter(|d| *d < 0)
        .sum();
    assert_eq!(
        debited, -100,
        "the outsider's 500 is not in the payout total"
    );
    // 4. …and so does the refund, which frees only what this contract reserved.
    let cancelled = EconomicContract {
        state: ContractState::Cancelled,
        ..c.clone()
    };
    let r = refund_plan(&cancelled, &s).expect("refund plan");
    assert_eq!(r.hold_ops, vec![HoldOp::Release { hold: HoldId(1) }]);

    // And a consideration with no qualifying hold behind it cannot settle at all.
    let unfunded = EconomicContract {
        escrow: vec![HoldId(7)],
        ..c
    };
    assert_eq!(settle(&unfunded, &s), Err(ContractError::EscrowIncomplete));
}

#[test]
fn vf_4_a_stale_allocator_floor_never_re_mints_a_live_hold() {
    let mut s = snap();
    with_hold(&mut s, hold(5, 1, 100));
    let k = Seams::open();
    let stale = ValidateEnv {
        keys: &mush_economy::MapOnlyKeys,
        auth: &k.auth,
        custody: &k.custody,
        facts: &k.facts,
        now: NOW,
        ic_now: IC,
        // The caller's floor has fallen behind the journal (E4b1 derives it from the
        // journal for exactly this reason).
        next_hold_id: 5,
        spent: std::collections::BTreeMap::new(),
    };
    assert_eq!(
        validate(&hold_plan("re", vec![place(1, 10)]), &s, &stale),
        PlanResult::Refused(EconomyError::HoldIdReused(HoldId(5)))
    );
    // …and the refusal changed nothing: the live reservation is untouched.
    let live = s.hold(HoldId(5)).unwrap();
    assert_eq!(live.status, HoldStatus::Held);
    assert_eq!((live.amount, live.revision), (Amount::from_minor(100), 1));
    assert_eq!(
        s.account(AccountId(1)).unwrap().held_amount,
        Amount::from_minor(100)
    );
}

#[test]
fn vf_5_a_canon_attestation_must_cite_the_right_canon_record() {
    let (s, mut c) = contract_snap();
    let k = Seams::open();
    let canon = |kind: &str, source: &str| {
        att(AttestorRef::Canon {
            canon: "canon:9".into(),
            kind: kind.into(),
            source: source.into(),
        })
    };
    c.consent.conditions = vec![TypedCondition::Attestation {
        by: AttestorSet::Canon {
            kind: "op.complete".into(),
            source: Some("op-7".into()),
        },
    }];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);

    // E5's owner answer 20: progress cites a SPECIFIC operation. A different
    // operation of the same kind is not this operation.
    c.satisfied = vec![Some(canon("op.complete", "op-8"))];
    assert_eq!(
        reverify(&c, &s, &k.env(&s)),
        Err(ContractError::ConditionFailed(0))
    );
    // Nor is a different kind naming the right source.
    c.satisfied = vec![Some(canon("op.failed", "op-7"))];
    assert_eq!(
        reverify(&c, &s, &k.env(&s)),
        Err(ContractError::ConditionFailed(0))
    );
    // The exact citation settles.
    c.satisfied = vec![Some(canon("op.complete", "op-7"))];
    assert!(reverify(&c, &s, &k.env(&s)).is_ok());

    // A set that names no source accepts any source OF THAT KIND, and no other.
    c.consent.conditions = vec![TypedCondition::Attestation {
        by: AttestorSet::Canon {
            kind: "op.complete".into(),
            source: None,
        },
    }];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    c.satisfied = vec![Some(canon("op.complete", "op-99"))];
    assert!(reverify(&c, &s, &k.env(&s)).is_ok());
    c.satisfied = vec![Some(canon("op.failed", "op-99"))];
    assert_eq!(
        reverify(&c, &s, &k.env(&s)),
        Err(ContractError::ConditionFailed(0))
    );
}

#[test]
fn vf_6_settlement_re_verifies_and_a_refund_needs_a_terminal_state() {
    let (s, mut c) = contract_snap();
    let fact = mush_economy::ExternalRef {
        provider: "jobs".into(),
        kind: "resolved".into(),
        id: "7".into(),
        revision: None,
    };
    c.consent.conditions = vec![TypedCondition::Fact { fact }];
    c.satisfied = vec![Some(att(AttestorRef::Clock))];
    reoffer(&mut c);

    // There is no way to get the plan without the proof: an unanswerable fact is
    // unsatisfied, so no settlement exists to be posted by mistake.
    assert_eq!(settle(&c, &s), Err(ContractError::ConditionFailed(0)));
    let mut yes = Seams::open();
    yes.facts.answer = Some(true);
    assert!(settlement_plan(&c, &s, &yes.env(&s)).is_ok());

    // A refund releases escrow, so it follows the state change rather than causing
    // one: on a live contract it would strip the funding out from under a standing
    // agreement, and on a settled one there is nothing left to give back.
    for state in [
        ContractState::Active,
        ContractState::ReadyToSettle,
        ContractState::Offered,
        ContractState::Settled {
            transaction: TransactionId(3),
        },
    ] {
        let live = EconomicContract { state, ..c.clone() };
        assert_eq!(
            refund_plan(&live, &s),
            Err(ContractError::WrongState(ContractId(1))),
            "{state:?}"
        );
    }
    for state in [ContractState::Cancelled, ContractState::Expired] {
        let done = EconomicContract { state, ..c.clone() };
        assert!(refund_plan(&done, &s).is_ok(), "{state:?}");
    }
}

#[test]
fn vf_7_every_hold_op_needs_an_active_account() {
    let mut s = snap();
    with_hold(&mut s, hold(1, 1, 100));
    let held = s.account(AccountId(1)).unwrap().clone();
    s.put_account(Account {
        status: AccountStatus::Frozen,
        ..held
    });
    // A freeze is what a staffer does to STOP an account's numbers moving, and every
    // money line already refuses there. An effect-only release or expiry would be
    // the one remaining way to keep changing `held_amount` on a frozen account.
    for op in [
        HoldOp::Release { hold: HoldId(1) },
        HoldOp::Capture {
            hold: HoldId(1),
            amount: Amount::from_minor(1),
        },
    ] {
        assert_eq!(
            refuse(&s, &hold_plan("fr", vec![op])),
            EconomyError::InactiveAccount(AccountId(1))
        );
    }
}

#[test]
fn vf_8_the_holder_key_is_a_label_and_never_authority() {
    let mut s = snap();
    // A hold whose `holder` names a contract's settlement key. The key is
    // caller-chosen — `post` is public — so this buys exactly nothing.
    with_hold(
        &mut s,
        Hold {
            holder: settle_key(ContractId(1)),
            ..hold(1, 1, 100)
        },
    );
    let mut plain = snap();
    with_hold(&mut plain, hold(1, 1, 100));

    let mut denied = Seams::open();
    denied.auth.hold = false;
    let release = || hold_plan("h", vec![HoldOp::Release { hold: HoldId(1) }]);
    assert_eq!(
        refuse_with(&s, &release(), &denied),
        EconomyError::Unauthorized
    );
    assert_eq!(
        refuse_with(&plain, &release(), &denied),
        EconomyError::Unauthorized
    );
    // …and with the seam saying yes, the two behave identically: nothing branches
    // on `holder`, so the effects match down to the status and the projection.
    let forged = prepare(&s, &release(), &Seams::open());
    let ordinary = prepare(&plain, &release(), &Seams::open());
    assert_eq!(
        forged.hold_effects[0].hold.status,
        ordinary.hold_effects[0].hold.status
    );
    assert_eq!(forged.balance_deltas, ordinary.balance_deltas);
}

#[test]
fn vf_9_advance_is_ordered_and_never_drops_a_settlement() {
    let mut o = obligation();
    o = advanced(&o, 0, SettleOutcome::Refused);
    o = advanced(&o, 1, SettleOutcome::Refused);
    assert_eq!(o.status, ObligationStatus::Delinquent);

    // A late payment on a delinquent obligation is real money: the period closes and
    // the settlement is recorded. The STATUS is a deliberate act (E4b1's
    // `reinstate`, or E5's hook), so it does not silently rejoin the due pool.
    let paid = advance_obligation(&o, 2, SettleOutcome::Posted(TransactionId(9)), NOW).unwrap();
    assert!(matches!(
        paid.history.last(),
        Some(ObligationEvent::Settled { period: 2, .. })
    ));
    assert_eq!((paid.periods_settled, paid.settled_total), (3, 1));
    assert_eq!(paid.status, ObligationStatus::Delinquent);

    // An outcome for a period that has not come round yet is refused outright.
    assert_eq!(
        advance_obligation(&o, 5, SettleOutcome::Posted(TransactionId(1)), NOW),
        Err(ObligationError::OutOfOrder)
    );

    // Which is also what keeps `plan_sweep`'s contract honest: its `attempt` list
    // starts PAST the periods it asked to have forgiven, so a caller who skips the
    // forgiveness step is refused instead of quietly swallowing it.
    let due = Obligation {
        schedule: EconomicSchedule::new(EconomicClock::RealUtc, 1_000, Some(100), 3),
        ..obligation()
    };
    let intent = plan_sweep(&due, 1_000 + 100 * 9);
    assert_eq!(intent.attempt[0], 7);
    assert_eq!(
        advance_obligation(
            &due,
            intent.attempt[0],
            SettleOutcome::Posted(TransactionId(1)),
            NOW
        ),
        Err(ObligationError::OutOfOrder)
    );
    let forgiven = forgive(
        &due,
        intent.forgive_from,
        intent.forgive_to,
        ForgiveCause::Downtime,
        NOW,
    );
    assert!(
        advance_obligation(
            &forgiven,
            intent.attempt[0],
            SettleOutcome::Posted(TransactionId(1)),
            NOW
        )
        .is_ok()
    );
}

#[test]
fn vf_10_a_contract_condition_may_not_ride_a_manual_clock() {
    let (s, mut c) = contract_snap();
    let k = Seams::open();
    c.consent.conditions = vec![TypedCondition::Before {
        deadline: EconomicDeadline {
            clock: EconomicClock::Manual,
            at: NOW + 1_000,
        },
    }];
    c.satisfied = vec![Some(att(AttestorRef::Clock))];
    reoffer(&mut c);
    // A manual clock's position is carried by the record that names it, and a
    // condition carries none — so the deadline would be read against whatever the
    // caller passed. Refused where a human can still fix the draft…
    assert_eq!(
        check_offerable(&c, &s),
        Err(ContractError::UnsupportedClock(0))
    );
    // …and, for a hand-built or migrated record that got past Offer, `reverify`
    // fails CLOSED: the unresolvable deadline reads as long passed.
    assert_eq!(
        reverify(&c, &s, &k.env(&s)),
        Err(ContractError::ConditionFailed(0))
    );
    assert_eq!(settle(&c, &s), Err(ContractError::UnsupportedClock(0)));

    let mut after = c;
    after.consent.conditions = vec![
        TypedCondition::Attestation {
            by: AttestorSet::Party(who(2)),
        },
        TypedCondition::After {
            condition: 0,
            delay: 10,
            clock: EconomicClock::Manual,
        },
    ];
    after.satisfied = vec![None, None];
    after.consent_hash = consent_hash(&after.issuer, &after.consent);
    assert_eq!(
        check_offerable(&after, &s),
        Err(ContractError::UnsupportedClock(1))
    );
}

#[test]
fn vf_11_a_one_shot_that_misses_goes_straight_to_delinquent() {
    let o = Obligation {
        schedule: EconomicSchedule::new(EconomicClock::RealUtc, 1_000, None, 3),
        // A generous grace allowance changes nothing: there is no NEXT period to
        // grace into, so the deadline would be an allowance that never expires.
        grace: GracePolicy { periods: 5 },
        ..obligation()
    };
    let missed = advanced(&o, 0, SettleOutcome::InsufficientAvailable);
    assert_eq!(missed.status, ObligationStatus::Delinquent);
    assert_eq!((missed.periods_settled, missed.missed_total), (1, 1));
    assert!(
        plan_sweep(&missed, i64::MAX).is_empty(),
        "and it leaves the due pool"
    );
}

#[test]
fn vf_12_a_cadence_clamps_its_cap_and_a_manual_one_never_self_advances() {
    let base = CadenceClock {
        id: mush_economy::CadenceId(1),
        name: "season".into(),
        source: CadenceSource::RealUtc,
        period: 100,
        catch_up_cap: 0,
        position: 0,
        advanced_at: 0,
        revision: 1,
        generation: mush_economy::CADENCE_GENERATION_INITIAL,
        schema: mush_economy::CADENCE_SCHEMA_LEGACY,
    };
    // A stored cap of 0 would take no periods at all while `advanced_at` still
    // moved: every period silently swallowed, forever.
    let (next, forgiven) = advance_cadence(&base, 1_000);
    assert_eq!((next.position, forgiven), (1, 9));

    // A manual cadence is a human's to move. Reading a wall instant as its source
    // would advance a clock nobody asked to advance — and every schedule on it.
    let manual = CadenceClock {
        source: CadenceSource::Manual,
        catch_up_cap: 3,
        ..base
    };
    assert_eq!(advance_cadence(&manual, 1_000_000), (manual.clone(), 0));
    assert_eq!(mush_economy::advance_cadence_manual(&manual, 2).position, 2);
}

#[test]
fn vf_13_a_period_index_at_the_ceiling_forgives_instead_of_colliding() {
    let schedule = EconomicSchedule::new(EconomicClock::RealUtc, 1_000, Some(100), 3);
    let now = 1_000 + 100 * 1_000;
    let o = Obligation {
        schedule,
        periods_settled: u32::MAX - 2,
        ..obligation()
    };

    // Saturating into the key index would hand several periods the same
    // `obligation/{id}/{period}` key, and the second post would resolve to
    // `AlreadyPosted` against the first — a period paid by another period's money.
    let intent = plan_sweep(&o, now);
    assert!(intent.reanchor);
    assert!(intent.attempt.is_empty(), "no index is consumed");
    assert_eq!(intent.forgive, due_periods(&schedule, now));
    assert_eq!(intent.forgive_from, intent.forgive_to);

    let after = forgive_all(&o, intent.forgive, ForgiveCause::Downtime, now);
    assert_eq!(
        after.periods_settled,
        u32::MAX - 2,
        "the key base never moved"
    );
    assert_eq!(after.forgiven_total, intent.forgive);
    assert!(
        after.schedule.next_due > now,
        "re-anchored to the first due instant after now"
    );
    assert_eq!(
        plan_sweep(&after, now),
        SweepIntent::default(),
        "and nothing is due again"
    );

    // The case that still FITS is still attempted, and its keys are distinct.
    let tight = Obligation {
        schedule,
        periods_settled: u32::MAX - 4,
        ..obligation()
    };
    let ok = plan_sweep(&tight, 1_000 + 100 * 3);
    assert!(!ok.reanchor);
    let keys: BTreeSet<u32> = ok.attempt.iter().copied().collect();
    assert_eq!(keys.len(), ok.attempt.len(), "no two periods share a key");
    assert_eq!(ok.attempt, vec![u32::MAX - 3, u32::MAX - 2, u32::MAX - 1]);
}

#[test]
fn vf_14a_the_issuer_is_part_of_what_a_party_agreed_to() {
    let (s, c) = contract_snap();
    assert_ne!(
        consent_hash(&who(1), &c.consent),
        consent_hash(&who(2), &c.consent),
        "who offered it decides who answers for it"
    );
    let swapped = EconomicContract {
        issuer: who(2),
        ..c
    };
    assert!(!swapped.consent_intact());
    assert!(!swapped.accepted_by(&who(1)));
    assert_eq!(settle(&swapped, &s), Err(ContractError::ConsentMismatch));
}

#[test]
fn vf_14b_a_cycle_among_after_anchors_is_refused_at_offer() {
    let (s, mut c) = contract_snap();
    let after = |condition: usize| TypedCondition::After {
        condition,
        delay: 1,
        clock: EconomicClock::RealUtc,
    };
    // Neither can ever be satisfied, so the contract is unsettleable and its escrow
    // is stuck until expiry — with nothing anywhere saying why.
    c.consent.conditions = vec![after(1), after(0)];
    c.satisfied = vec![None, None];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    assert_eq!(
        check_offerable(&c, &s),
        Err(ContractError::BadConditionRef(0))
    );

    // A chain that terminates on a real condition is fine.
    c.consent.conditions = vec![
        TypedCondition::Attestation {
            by: AttestorSet::Party(who(2)),
        },
        after(0),
        after(1),
    ];
    c.satisfied = vec![None, None, None];
    c.consent_hash = consent_hash(&c.issuer, &c.consent);
    assert!(check_offerable(&c, &s).is_ok());
}

#[test]
fn vf_14c_a_rewound_sweep_stamp_is_a_time_jump_not_an_overload() {
    // `Overload` escalates when it repeats, so reading an NTP step or a restored
    // backup as a starved post budget would page a human about nothing.
    assert_eq!(
        classify_forgiveness(1_000, 1_000, 999, 999, 1, false),
        ForgiveCause::TimeJump
    );
    assert_eq!(
        classify_forgiveness(1_000, 1_000, 0, 0, 1, false),
        ForgiveCause::TimeJump
    );
    // …while a genuine "sweeps ran, this one got no budget" is untouched.
    assert_eq!(
        classify_forgiveness(0, 0, 30, 30, 1, false),
        ForgiveCause::Overload
    );
}

#[test]
fn vf_14d_the_tallies_outlive_the_bounded_history_tail() {
    let mut o = obligation();
    let runs = mush_economy::MAX_OBLIGATION_HISTORY as u32 + 10;
    for period in 0..runs {
        o = advanced(&o, period, SettleOutcome::Posted(TransactionId(1)));
    }
    assert_eq!(o.history.len(), mush_economy::MAX_OBLIGATION_HISTORY);
    // "Has this tenant ever paid?" survives the FIFO eviction that ate the evidence.
    assert_eq!(o.settled_total, runs);
    let missed = advanced(&o, o.periods_settled, SettleOutcome::Refused);
    assert_eq!((missed.settled_total, missed.missed_total), (runs, 1));
    let forgiven = forgive(
        &missed,
        missed.periods_settled,
        missed.periods_settled + 5,
        ForgiveCause::Downtime,
        NOW,
    );
    assert_eq!(
        (
            forgiven.settled_total,
            forgiven.missed_total,
            forgiven.forgiven_total
        ),
        (runs, 1, 5)
    );
}
