#![cfg(test)]

//! Doc 47 CO.13: the closed stored-core sale plan's guard, its receipt and its audit, driven
//! through the real engine (`validate_core_sale`, `apply_delta`) over a snapshot holding one
//! managed core, the seller's and the vendor's accounts. The participant's prepare is the
//! composed story's to drive (`tacsim` `domain_field` `c4_hardware::co13_*`).
use super::*;
use crate::engine::MapOnlyKeys;
use crate::ids::InstrumentId;
use crate::record::{AssetLifecycle, CreditPolicy, InstrumentKind, NegativePolicy};
use mush_org::OrgId;
use mush_testkit::fixtures::token as commit;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const CORE: ObjId = ObjId(70);
const SELLER: ObjId = ObjId(50);
const OTHER: ObjId = ObjId(51);
const SHOP: OrgId = OrgId(4);
const PAYOUT: AccountId = AccountId(30);
const TILL: AccountId = AccountId(31);
const PRICE: i64 = 400_000;

fn issuer() -> SystemPrincipalId {
    SystemPrincipalId::new("launch.issuer").expect("an issuer")
}
fn birth() -> crate::managed_title::TitleBirth {
    crate::managed_title::TitleBirth::core(commit(1), TransactionId(1))
}
fn vendor() -> ScopedCoreAccount {
    ScopedCoreAccount {
        account: TILL,
        organization: SHOP,
        scope: OrgScopeId(5),
    }
}
fn config() -> CoreSaleConfig {
    let action = mush_core::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(ACTION_OWNER).expect("an owner"),
            mush_core::DefinitionDescriptor::new(
                mush_core::DefinitionKind::new("action").expect("a kind"),
                mush_core::DefinitionId::new(ACTION_ID).expect("an id"),
                1,
                serde_json::json!({"fixture": ACTION_ID}),
            )
            .expect("a descriptor"),
        )
        .expect("registers");
    CoreSaleConfig {
        action,
        vendor: vendor(),
    }
}
fn policy() -> InitialFilingPolicy {
    InitialFilingPolicy::new(
        issuer(),
        AccountId(2),
        Amount::ZERO,
        CatalogRef {
            ruleset: "roll".into(),
            code: "charter".into(),
        },
    )
    .expect("a policy")
}
fn core_title(owner: ObjId) -> EconomicAsset {
    EconomicAsset {
        object: CORE,
        title_owner: PrincipalId::Object(owner),
        scope: None,
        assignment: None,
        maintenance_account: None,
        catalog: Some(CatalogRef {
            ruleset: "tacsim".into(),
            code: "tacsim.core.line".into(),
        }),
        encumbrances: Vec::new(),
        acquisition: None,
        lifecycle: AssetLifecycle::Active,
        last_journal_seq: 3,
        revision: 4,
        stolen: None,
        frozen: false,
        frozen_reason: None,
    }
}
fn credit() -> InstrumentId {
    InstrumentId::new("tacsim.credit").expect("an instrument")
}
fn account(id: AccountId, owner: PrincipalId, scope: Option<OrgScopeId>, balance: i64) -> Account {
    Account {
        id,
        owner,
        instrument: credit(),
        scope,
        name: format!("account-{}", id.0).into(),
        status: AccountStatus::Active,
        posted_balance: Amount::from_minor(balance),
        held_amount: Amount::ZERO,
        credit: CreditPolicy::forbid(),
        last_journal_seq: 0,
        last_hold_seq: 0,
        revision: 1,
        approval: None,
    }
}
fn snapshot(owner: ObjId) -> EconomySnapshot {
    let mut snapshot = EconomySnapshot::default();
    snapshot.put_instrument(crate::record::InstrumentDef {
        id: credit(),
        ruleset: "tacsim".into(),
        code: "cr".into(),
        kind: InstrumentKind::Currency,
        scale: 2,
        transferable: true,
        negative_policy: NegativePolicy::SystemOnly,
        status: InstrumentStatus::Active,
        display: None,
        revision: 1,
    });
    snapshot.put_account(account(PAYOUT, PrincipalId::Object(SELLER), None, 0));
    snapshot.put_account(account(
        TILL,
        PrincipalId::Organization(SHOP),
        Some(OrgScopeId(5)),
        9_000_000,
    ));
    snapshot.put_asset(core_title(owner));
    snapshot.managed_titles =
        crate::managed_title::ManagedTitleIndex::Ready(BTreeMap::from([(CORE, birth())]));
    snapshot
}
fn request(revision: u64) -> RegistryCoreSale {
    RegistryCoreSale {
        core: CORE,
        title_revision: revision,
        seller_account: PAYOUT,
        price: Amount::from_minor(PRICE),
    }
}
fn closed(snapshot: &EconomySnapshot, n: u8, r: &RegistryCoreSale) -> ClosedCoreSalePlan {
    ClosedCoreSalePlan {
        plan: plan(&config(), &policy(), (SELLER, commit(n), 2_000), r),
        before_title: snapshot.asset(CORE).cloned().expect("the title"),
        birth: birth(),
        vendor: PrincipalId::Organization(SHOP),
        lines: lines(&config(), r),
    }
}
/// Validate and apply one closed plan through the real engine, as the participant does.
fn post(
    snapshot: &mut EconomySnapshot,
    proof: &ClosedCoreSalePlan,
    r: &RegistryCoreSale,
    id: u64,
) -> std::result::Result<EconomicTransaction, crate::EconomyError> {
    let auth = SaleAuth {
        issuer: PrincipalId::System(issuer()),
        vendor: PrincipalId::Organization(SHOP),
        seller: SELLER,
        lines: lines(&config(), r),
        ops: ops(CORE, &PrincipalId::Organization(SHOP)),
        core: CORE,
        title_revision: r.title_revision,
    };
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: 1_000,
        ic_now: 2_000,
        next_hold_id: 1,
        spent: BTreeMap::new(),
        keys: &MapOnlyKeys,
    };
    match crate::engine::validate_core_sale(proof, snapshot, &env) {
        PlanResult::Post(prepared) => {
            let (delta, tx) =
                crate::engine::apply_delta(snapshot, &prepared, TransactionId(id), id, 1_000);
            delta.apply_to(snapshot);
            Ok(tx)
        }
        PlanResult::Refused(error) => Err(error),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) => {
            Err(crate::EconomyError::Unauthorized)
        }
    }
}

/// **The plan proof**: exactly the two lines and the one transfer to the vendor org; any other
/// plan, title image or birth is refused.
#[test]
fn co13_the_closed_plan_refuses_every_mismatch() -> Result {
    let snapshot = snapshot(SELLER);
    let r = request(4);
    let good = closed(&snapshot, 5, &r);
    good.check(good.plan(), &snapshot)?;
    check_eq!(
        good.plan().asset_ops,
        vec![AssetOp::TransferTitle {
            object: CORE,
            to: PrincipalId::Organization(SHOP),
            to_scope: None,
        }],
        "exactly one transfer, to the vendor org"
    );
    type Change = Box<dyn Fn(&mut EconomyPlan)>;
    let plan_changes: [(&str, Change); 9] = [
        (
            "a second op",
            Box::new(|p| p.asset_ops.push(AssetOp::Unassign { object: CORE })),
        ),
        (
            "another buyer",
            Box::new(|p| {
                p.asset_ops = vec![AssetOp::TransferTitle {
                    object: CORE,
                    to: PrincipalId::Object(OTHER),
                    to_scope: None,
                }];
            }),
        ),
        (
            "a scope on the transfer",
            Box::new(|p| {
                p.asset_ops = vec![AssetOp::TransferTitle {
                    object: CORE,
                    to: PrincipalId::Organization(SHOP),
                    to_scope: Some(OrgScopeId(5)),
                }];
            }),
        ),
        (
            "a dearer price",
            Box::new(|p| p.requested_lines[1].delta = Amount::from_minor(PRICE + 1)),
        ),
        (
            "a third line",
            Box::new(|p| {
                p.requested_lines.push(RequestedLine {
                    account: AccountId(2),
                    delta: Amount::from_minor(1),
                    memo: "x".into(),
                });
            }),
        ),
        (
            "another namespace",
            Box::new(|p| p.idempotency = IdempotencyKey::new("roll.charter_transfer.v1", "x")),
        ),
        (
            "another actor",
            Box::new(|p| p.actor = PrincipalId::Object(OTHER)),
        ),
        (
            "no revision fence",
            Box::new(|p| p.expected_asset_revisions.clear()),
        ),
        (
            "an encumbrance instead",
            Box::new(|p| {
                p.asset_ops = vec![AssetOp::Encumber {
                    object: CORE,
                    encumbrance: crate::record::Encumbrance::Disputed {
                        claimant: PrincipalId::Organization(SHOP),
                        reason: "x".into(),
                    },
                }];
            }),
        ),
    ];
    for (why, change) in plan_changes {
        let mut plan = good.plan().clone();
        change(&mut plan);
        check!(good.check(&plan, &snapshot).is_err(), "{why}");
    }
    let mut moved = snapshot.clone();
    let mut title = core_title(SELLER);
    title.revision = 5;
    moved.put_asset(title);
    check!(
        good.check(good.plan(), &moved).is_err(),
        "a wrong title image"
    );
    for index in [
        BTreeMap::from([(
            CORE,
            crate::managed_title::TitleBirth::core(commit(2), TransactionId(1)),
        )]),
        BTreeMap::from([(
            CORE,
            crate::managed_title::TitleBirth::charter(commit(1), TransactionId(1)),
        )]),
        BTreeMap::new(),
    ] {
        let mut other = snapshot.clone();
        other.managed_titles = crate::managed_title::ManagedTitleIndex::Ready(index);
        check!(good.check(good.plan(), &other).is_err(), "a wrong birth");
    }
    Ok(())
}

/// **No generic path moves a managed title**: the sale's exact plan through the generic guard
/// is refused, `require_unmanaged_mutations` refuses its transfer, and the same transfer of an
/// unmanaged object passes it.
#[test]
fn co13_no_generic_transfer_touches_a_managed_title() -> Result {
    let snapshot = snapshot(SELLER);
    let r = request(4);
    let good = closed(&snapshot, 5, &r);
    check!(
        crate::managed_title::require_unmanaged_mutations(good.plan(), &snapshot).is_err(),
        "the sale's transfer is refused outside its closed plan"
    );
    let auth = SaleAuth {
        issuer: PrincipalId::System(issuer()),
        vendor: PrincipalId::Organization(SHOP),
        seller: SELLER,
        lines: lines(&config(), &r),
        ops: ops(CORE, &PrincipalId::Organization(SHOP)),
        core: CORE,
        title_revision: 4,
    };
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: 1_000,
        ic_now: 2_000,
        next_hold_id: 1,
        spent: BTreeMap::new(),
        keys: &MapOnlyKeys,
    };
    check!(
        matches!(
            crate::engine::validate(good.plan(), &snapshot, &env),
            PlanResult::Refused(_)
        ),
        "the generic post refuses the exact sale plan"
    );
    let mut unmanaged = snapshot.clone();
    unmanaged.managed_titles = crate::managed_title::ManagedTitleIndex::Ready(BTreeMap::new());
    crate::managed_title::require_unmanaged_mutations(good.plan(), &unmanaged)?;
    Ok(())
}

/// **The sale through the engine and the audit**: the vendor pays, the seller is paid, the title
/// is the vendor org's; the receipt replays, and a forged entry in the family's namespace marks
/// the audit invalid.
#[test]
fn co13_a_sale_posts_replays_and_a_forgery_is_refused() -> Result {
    let mut snapshot = snapshot(SELLER);
    let r = request(4);
    let sale = closed(&snapshot, 5, &r);
    let tx = post(&mut snapshot, &sale, &r, 7)?;
    let title = snapshot.asset(CORE).cloned().ok_or("the title")?;
    check_eq!(title.title_owner, PrincipalId::Organization(SHOP));
    check!(title.encumbrances.is_empty());
    check_eq!(
        snapshot.account(PAYOUT).map(|a| a.posted_balance.minor()),
        Some(PRICE)
    );
    check_eq!(
        snapshot.account(TILL).map(|a| a.posted_balance.minor()),
        Some(9_000_000 - PRICE)
    );
    let subject = receipt::subject_of(&tx)?;
    check_eq!(
        (
            subject.seller,
            subject.core,
            subject.vendor,
            subject.price.minor()
        ),
        (SELLER, CORE, SHOP, PRICE)
    );
    check_eq!(subject.title, title);
    let mut audit = CoreSaleReceiptAudit::default();
    check!(
        audit.require_complete().is_err(),
        "not complete before replay"
    );
    audit.observe(&tx);
    audit.finish_replay()?;
    check_eq!(audit.receipts()?.len(), 1);
    audit.verify_titles(&snapshot.managed_titles)?;
    check!(
        audit
            .verify_titles(&crate::managed_title::ManagedTitleIndex::Ready(
                BTreeMap::new()
            ))
            .is_err(),
        "a sale of no managed core is refused"
    );

    // Forgeries in the family's namespace: a richer price on the correlation, a third line, a
    // second commit of the same entry.
    let forged_price = {
        let mut tx = tx.clone();
        if let EconomicCause::Ruleset { correlation, .. } = &mut tx.cause {
            *correlation = correlation.replace(":400000:", ":400001:").into();
        }
        tx
    };
    let third_line = {
        let mut tx = tx.clone();
        let mut line = tx.lines[1].clone();
        line.account = AccountId(2);
        tx.lines.push(line);
        tx
    };
    let elsewhere = {
        let mut tx = tx.clone();
        for line in &mut tx.lines {
            if line.account == PAYOUT {
                line.account = AccountId(2);
            }
        }
        tx
    };
    for (why, forged) in [
        ("price", forged_price),
        ("line", third_line),
        ("the seller paid elsewhere", elsewhere),
    ] {
        check!(receipt::subject_of(&forged).is_err(), "{why}");
        let mut audit = CoreSaleReceiptAudit::default();
        audit.observe(&forged);
        check!(audit.finish_replay().is_err(), "{why}");
    }
    let mut twice = CoreSaleReceiptAudit::default();
    twice.observe(&tx);
    twice.observe(&tx);
    check!(twice.finish_replay().is_err(), "one commit, one sale");
    Ok(())
}

/// **The title refusals the engine enforces under the closed plan**: a core titled to someone
/// else and a core carrying a claim are not sold.
#[test]
fn co13_another_holders_or_a_claimed_title_is_refused() -> Result {
    let mut theirs = snapshot(OTHER);
    let r = request(4);
    let sale = closed(&theirs, 5, &r);
    check!(post(&mut theirs, &sale, &r, 7).is_err(), "another's title");
    let mut claimed = snapshot(SELLER);
    let mut title = core_title(SELLER);
    title
        .encumbrances
        .push(crate::record::Encumbrance::Disputed {
            claimant: PrincipalId::Organization(OrgId(9)),
            reason: "hull/0000003c/inspection/3".into(),
        });
    claimed.put_asset(title);
    let sale = closed(&claimed, 6, &r);
    check!(post(&mut claimed, &sale, &r, 8).is_err(), "a claimed title");
    Ok(())
}

/// **The prepare's read** (`title`): the seller's own unencumbered managed core, their own payout
/// account and the configured till pass; another holder's core, a claimed one, a payout account
/// that is not the seller's and a zero price are each refused.
#[test]
fn co13_the_prepare_read_refuses_what_the_seller_may_not_sell() -> Result {
    let mut tw = mush_testkit::TestWorld::with_modules(Vec::new());
    let core = tw.spawn_player("a live stand-in for the core");
    let mut snap = snapshot(SELLER);
    let mut title = core_title(SELLER);
    title.object = core;
    snap.put_asset(title.clone());
    snap.managed_titles =
        crate::managed_title::ManagedTitleIndex::Ready(BTreeMap::from([(core, birth())]));
    let store = crate::EconomyStore {
        title_issuers: std::sync::Arc::new(std::collections::BTreeSet::from([issuer()])),
        snapshot: snap,
        ..crate::EconomyStore::default()
    };
    let ask = |r: &RegistryCoreSale, store: &crate::EconomyStore| {
        super::title(&config(), &policy(), store, tw.world(), SELLER, r).map(|_| ())
    };
    let good = RegistryCoreSale { core, ..request(4) };
    check!(ask(&good, &store).is_ok(), "the seller's own core");
    let mut theirs = store.clone();
    let mut other = title.clone();
    other.title_owner = PrincipalId::Object(OTHER);
    theirs.snapshot.put_asset(other);
    check!(ask(&good, &theirs).is_err(), "another holder's core");
    let mut claimed = store.clone();
    let mut held = title.clone();
    held.encumbrances
        .push(crate::record::Encumbrance::Disputed {
            claimant: PrincipalId::Organization(OrgId(9)),
            reason: "x".into(),
        });
    claimed.snapshot.put_asset(held);
    check!(ask(&good, &claimed).is_err(), "a claimed core");
    let mut payee = store.clone();
    payee
        .snapshot
        .put_account(account(PAYOUT, PrincipalId::Object(OTHER), None, 0));
    check!(
        ask(&good, &payee).is_err(),
        "a payout account not the seller's"
    );
    let free = RegistryCoreSale {
        price: Amount::ZERO,
        ..good.clone()
    };
    check!(ask(&free, &store).is_err(), "a zero price");
    Ok(())
}
