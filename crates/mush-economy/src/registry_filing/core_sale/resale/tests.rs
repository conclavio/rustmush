#![cfg(test)]

//! Doc 48 unit EX.2b: the closed governed-core resale plan's guard, its receipt and its audit,
//! driven through the real engine (`validate_core_resale`, `apply_delta`) over a snapshot
//! holding one managed core, an unmanaged hull, her hold, and the buyer's, the seller's, the
//! venue's and the levy's accounts. The participant's prepare is the composed story's to drive
//! (`tacsim` `domain_field` `ex2b_core_resale`).
use super::*;
use crate::engine::MapOnlyKeys;
use crate::ids::InstrumentId;
use crate::record::{AssetLifecycle, CreditPolicy, CustodyRef, InstrumentKind, NegativePolicy};
use mush_org::OrgId;
use mush_testkit::fixtures::token as commit;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const CORE: ObjId = ObjId(70);
const HULL: ObjId = ObjId(71);
const SELLER: ObjId = ObjId(50);
const BUYER: ObjId = ObjId(52);
const OTHER: ObjId = ObjId(51);
const VENUE: OrgId = OrgId(4);
const PAYOUT: AccountId = AccountId(30);
const PURSE: AccountId = AccountId(32);
const CUT: AccountId = AccountId(33);
const SINK: AccountId = AccountId(34);
const HOLD: VaultId = VaultId(9);
const PRICE: i64 = 400_000;

fn issuer() -> SystemPrincipalId {
    SystemPrincipalId::new("launch.issuer").expect("an issuer")
}
fn sink() -> SystemPrincipalId {
    SystemPrincipalId::new("launch.registry").expect("a sink")
}
fn birth() -> crate::managed_title::TitleBirth {
    crate::managed_title::TitleBirth::core(commit(1), TransactionId(1))
}
fn config() -> CoreResaleConfig {
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
    CoreResaleConfig {
        action,
        venue: Recipient {
            account: CUT,
            owner: PrincipalId::Organization(VENUE),
        },
        levy: Recipient {
            account: SINK,
            owner: PrincipalId::System(sink()),
        },
        venue_permille: VENUE_PERMILLE,
        levy_permille: LEVY_PERMILLE,
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
fn title(object: ObjId, owner: ObjId, revision: u64) -> EconomicAsset {
    EconomicAsset {
        object,
        title_owner: PrincipalId::Object(owner),
        scope: None,
        assignment: None,
        maintenance_account: None,
        catalog: Some(CatalogRef {
            ruleset: "tacsim".into(),
            code: if object == CORE {
                "tacsim.core.line".into()
            } else {
                "hull.tacsim.unit.dray".into()
            },
        }),
        encumbrances: Vec::new(),
        acquisition: None,
        lifecycle: AssetLifecycle::Active,
        last_journal_seq: 3,
        revision,
        stolen: None,
        frozen: false,
        frozen_reason: None,
    }
}
fn credit() -> InstrumentId {
    InstrumentId::new("tacsim.credit").expect("an instrument")
}
fn account(id: AccountId, owner: PrincipalId, balance: i64) -> Account {
    Account {
        id,
        owner,
        instrument: credit(),
        scope: None,
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
    snapshot.put_account(account(PAYOUT, PrincipalId::Object(SELLER), 0));
    snapshot.put_account(account(PURSE, PrincipalId::Object(BUYER), 9_000_000));
    snapshot.put_account(account(CUT, PrincipalId::Organization(VENUE), 0));
    snapshot.put_account(account(SINK, PrincipalId::System(sink()), 0));
    snapshot.put_asset(title(CORE, owner, 4));
    snapshot.put_asset(title(HULL, owner, 6));
    snapshot.put_vault(Vault {
        id: HOLD,
        owner: PrincipalId::Object(owner),
        scope: None,
        name: "hold".into(),
        backing: CustodyRef::SpatialInterior(ObjId(72)),
        status: crate::record::VaultStatus::Active,
        revision: 1,
    });
    snapshot.managed_titles =
        crate::managed_title::ManagedTitleIndex::Ready(BTreeMap::from([(CORE, birth())]));
    snapshot
}
fn request(hull: bool) -> RegistryCoreResale {
    RegistryCoreResale {
        core: CORE,
        title_revision: 4,
        hull: hull.then_some(ResaleHull {
            hull: HULL,
            title_revision: 6,
            hold: Some(HOLD),
        }),
        seller: SELLER,
        seller_account: PAYOUT,
        buyer_account: PURSE,
        price: Amount::from_minor(PRICE),
    }
}
fn closed(snapshot: &EconomySnapshot, n: u8, r: &RegistryCoreResale) -> ClosedCoreResalePlan {
    ClosedCoreResalePlan {
        plan: plan(&config(), &policy(), (BUYER, commit(n), 2_000), r),
        before_title: snapshot.asset(CORE).cloned().expect("the title"),
        before_hull: r
            .hull
            .as_ref()
            .map(|leg| snapshot.asset(leg.hull).cloned().expect("her title")),
        birth: birth(),
        buyer: BUYER,
        lines: lines(&config(), r),
    }
}
fn auth(r: &RegistryCoreResale) -> ResaleAuth {
    let config = config();
    ResaleAuth {
        issuer: PrincipalId::System(issuer()),
        owners: [
            PrincipalId::Object(BUYER),
            PrincipalId::Object(SELLER),
            config.venue.owner.clone(),
            config.levy.owner.clone(),
        ],
        seller: SELLER,
        buyer: BUYER,
        lines: lines(&config, r),
        ops: ops(CORE, r.hull.as_ref().map(|leg| leg.hull), BUYER),
        revisions: revisions(r),
    }
}
/// Validate and apply one closed plan through the real engine, as the participant does.
fn post(
    snapshot: &mut EconomySnapshot,
    proof: &ClosedCoreResalePlan,
    r: &RegistryCoreResale,
    id: u64,
) -> std::result::Result<EconomicTransaction, crate::EconomyError> {
    let auth = auth(r);
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
    match crate::engine::validate_core_resale(proof, snapshot, &env) {
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

/// **The split** at the provisional rates: 3% to the venue, 1% to the sink, the rest to the
/// seller; four lines that sum to zero.
#[test]
fn ex2b_the_split_pays_seller_venue_and_levy() -> Result {
    let config = config();
    config.validate()?;
    check_eq!(
        config.split(Amount::from_minor(PRICE)),
        Some((12_000, 4_000, 384_000))
    );
    let four = lines(&config, &request(false));
    check_eq!(four.len(), 4);
    check_eq!(four.iter().map(|l| l.delta.minor()).sum::<i64>(), 0);
    check_eq!(config.split(Amount::ZERO), None, "no price");
    check_eq!(
        config.split(Amount::from_minor(10)),
        Some((0, 0, 10)),
        "a price too small for a cut has no cut"
    );
    check_eq!(
        lines(
            &config,
            &RegistryCoreResale {
                price: Amount::from_minor(10),
                ..request(false)
            }
        )
        .len(),
        2,
        "a zero cut has no line"
    );
    let mut person = config.clone();
    person.venue.owner = PrincipalId::Object(OTHER);
    check!(person.validate().is_err(), "a person is no venue");
    let mut org_sink = config.clone();
    org_sink.levy.owner = PrincipalId::Organization(VENUE);
    check!(org_sink.validate().is_err(), "the levy is a system sink");
    let mut greedy = config.clone();
    greedy.venue_permille = 990;
    check!(
        greedy.validate().is_err(),
        "the cuts leave the seller something"
    );
    Ok(())
}

/// **The plan proof**: exactly the composed lines and the transfers to the buyer; any other
/// plan, title image, birth or a managed hull is refused.
#[test]
fn ex2b_the_closed_plan_refuses_every_mismatch() -> Result {
    let snapshot = snapshot(SELLER);
    for hull in [false, true] {
        let r = request(hull);
        let good = closed(&snapshot, 5, &r);
        good.check(good.plan(), &snapshot)?;
        check_eq!(good.plan().asset_ops.len(), 1 + usize::from(hull));
        type Change = Box<dyn Fn(&mut EconomyPlan)>;
        let plan_changes: [(&str, Change); 10] = [
            (
                "a second op",
                Box::new(|p| p.asset_ops.push(AssetOp::Unassign { object: CORE })),
            ),
            (
                "another buyer",
                Box::new(|p| {
                    p.asset_ops[0] = AssetOp::TransferTitle {
                        object: CORE,
                        to: PrincipalId::Object(OTHER),
                        to_scope: None,
                    };
                }),
            ),
            (
                "a scope on the transfer",
                Box::new(|p| {
                    p.asset_ops[0] = AssetOp::TransferTitle {
                        object: CORE,
                        to: PrincipalId::Object(BUYER),
                        to_scope: Some(OrgScopeId(5)),
                    };
                }),
            ),
            (
                "a richer seller",
                Box::new(|p| p.requested_lines[1].delta = Amount::from_minor(PRICE)),
            ),
            (
                "no levy",
                Box::new(|p| {
                    p.requested_lines.pop();
                }),
            ),
            (
                "a fifth line",
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
                Box::new(|p| p.idempotency = IdempotencyKey::new("tacsim.core_sale.v1", "x")),
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
                "a hold rides along",
                Box::new(|p| {
                    p.hold_ops.push(crate::plan::HoldOp::Place {
                        account: PURSE,
                        amount: Amount::from_minor(1),
                        beneficiary: None,
                        purpose: crate::record::HoldPurpose::Reservation {
                            kind: "x".into(),
                            id: 1,
                        },
                        expires: None,
                    });
                }),
            ),
        ];
        for (why, change) in plan_changes {
            let mut plan = good.plan().clone();
            change(&mut plan);
            check!(good.check(&plan, &snapshot).is_err(), "{why} (hull {hull})");
        }
        let mut moved = snapshot.clone();
        moved.put_asset(title(CORE, SELLER, 5));
        check!(
            good.check(good.plan(), &moved).is_err(),
            "a wrong core image"
        );
        let mut index = BTreeMap::from([(CORE, birth())]);
        if hull {
            let mut moved = snapshot.clone();
            moved.put_asset(title(HULL, SELLER, 7));
            check!(
                good.check(good.plan(), &moved).is_err(),
                "a wrong hull image"
            );
            index.insert(HULL, birth());
            let mut managed = snapshot.clone();
            managed.managed_titles = crate::managed_title::ManagedTitleIndex::Ready(index);
            check!(
                good.check(good.plan(), &managed).is_err(),
                "a managed hull is no hull leg"
            );
        }
        for index in [
            BTreeMap::from([(
                CORE,
                crate::managed_title::TitleBirth::core(commit(2), TransactionId(1)),
            )]),
            BTreeMap::new(),
        ] {
            let mut other = snapshot.clone();
            other.managed_titles = crate::managed_title::ManagedTitleIndex::Ready(index);
            check!(good.check(good.plan(), &other).is_err(), "a wrong birth");
        }
    }
    Ok(())
}

/// **No generic path moves a managed title**: the resale's exact plan through the generic guard
/// is refused, and through the buy-back's proof too.
#[test]
fn ex2b_no_generic_transfer_touches_a_managed_title() -> Result {
    let snapshot = snapshot(SELLER);
    let r = request(true);
    let good = closed(&snapshot, 5, &r);
    check!(
        crate::managed_title::require_unmanaged_mutations(good.plan(), &snapshot).is_err(),
        "the resale's transfer is refused outside its closed plan"
    );
    let auth = auth(&r);
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
        "the generic post refuses the exact resale plan"
    );
    Ok(())
}

/// **A resale through the engine and the audit**, stored and with her hull: the buyer pays, the
/// seller, the venue and the sink are paid, both titles are the buyer's; the receipt replays,
/// and a forged entry in the family's namespace marks the audit invalid.
#[test]
fn ex2b_a_resale_posts_replays_and_a_forgery_is_refused() -> Result {
    for hull in [false, true] {
        let mut snapshot = snapshot(SELLER);
        let r = request(hull);
        let sale = closed(&snapshot, 5, &r);
        let tx = post(&mut snapshot, &sale, &r, 7)?;
        let core = snapshot.asset(CORE).cloned().ok_or("the title")?;
        check_eq!(core.title_owner, PrincipalId::Object(BUYER));
        let her = snapshot.asset(HULL).cloned().ok_or("her title")?;
        check_eq!(
            her.title_owner,
            PrincipalId::Object(if hull { BUYER } else { SELLER }),
            "the hull moves only with a hull leg"
        );
        for (account, balance) in [
            (PURSE, 9_000_000 - PRICE),
            (PAYOUT, 384_000),
            (CUT, 12_000),
            (SINK, 4_000),
        ] {
            check_eq!(
                snapshot.account(account).map(|a| a.posted_balance.minor()),
                Some(balance),
                "account {}",
                account.0
            );
        }
        let subject = receipt::subject_of(&tx)?;
        check_eq!(
            (subject.buyer, subject.seller, subject.core, subject.hull),
            (BUYER, SELLER, CORE, hull.then_some(HULL))
        );
        check_eq!(
            (
                subject.proceeds().minor(),
                subject.venue_cut().minor(),
                subject.levy().minor()
            ),
            (384_000, 12_000, 4_000)
        );
        check_eq!(subject.title, core);
        check_eq!(subject.hull_title.as_ref(), hull.then_some(&her));
        let mut audit = CoreResaleReceiptAudit::default();
        check!(audit.require_complete().is_err());
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
            "a resale of no managed core is refused"
        );

        // Forgeries in the family's namespace: a richer price on the correlation, a fifth line,
        // the seller paid elsewhere, a levy skimmed into the venue's line.
        let forged_price = {
            let mut tx = tx.clone();
            if let EconomicCause::Ruleset { correlation, .. } = &mut tx.cause {
                *correlation = correlation.replace(":400000:", ":400001:").into();
            }
            tx
        };
        let fifth_line = {
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
        let skimmed = {
            let mut tx = tx.clone();
            tx.lines.retain(|l| l.account != SINK);
            for line in &mut tx.lines {
                if line.account == CUT {
                    line.delta = Amount::from_minor(16_000);
                }
            }
            tx
        };
        for (why, forged) in [
            ("price", forged_price),
            ("line", fifth_line),
            ("the seller paid elsewhere", elsewhere),
            ("the levy skimmed", skimmed),
        ] {
            check!(receipt::subject_of(&forged).is_err(), "{why}");
            let mut audit = CoreResaleReceiptAudit::default();
            audit.observe(&forged);
            check!(audit.finish_replay().is_err(), "{why}");
        }
        let mut twice = CoreResaleReceiptAudit::default();
        twice.observe(&tx);
        twice.observe(&tx);
        check!(twice.finish_replay().is_err(), "one commit, one resale");
    }
    Ok(())
}

/// **Both or neither** (Q-48-52): a hull leg whose title is not the seller's at the offer's
/// revision refuses the whole plan, and the core does not move either.
#[test]
fn ex2b_a_cored_hull_sells_with_her_core_or_neither_moves() -> Result {
    let r = request(true);
    let mut moved = snapshot(SELLER);
    moved.put_asset(title(HULL, OTHER, 6));
    let sale = closed(&moved, 5, &r);
    check!(
        post(&mut moved, &sale, &r, 7).is_err(),
        "her title is another's"
    );
    check_eq!(
        moved.asset(CORE).map(|t| t.title_owner.clone()),
        Some(PrincipalId::Object(SELLER)),
        "the core stays with her"
    );
    check_eq!(
        moved.account(PURSE).map(|a| a.posted_balance.minor()),
        Some(9_000_000),
        "and nobody paid"
    );
    let mut claimed = snapshot(SELLER);
    let mut her = title(HULL, SELLER, 6);
    her.encumbrances.push(crate::record::Encumbrance::Disputed {
        claimant: PrincipalId::Organization(OrgId(9)),
        reason: "lien".into(),
    });
    claimed.put_asset(her);
    let sale = closed(&claimed, 6, &r);
    check!(post(&mut claimed, &sale, &r, 8).is_err(), "a claimed hull");
    check_eq!(
        claimed.asset(CORE).map(|t| t.title_owner.clone()),
        Some(PrincipalId::Object(SELLER))
    );
    Ok(())
}

/// **The prepare's read** (`titles`): the seller's own core (and hull, and hold) and the four
/// accounts pass; a buyer who is the seller, another holder's core, a claimed one, a hold that
/// is not the seller's, a purse that is not the buyer's and a zero price are each refused.
#[test]
fn ex2b_the_prepare_read_refuses_what_may_not_be_resold() -> Result {
    let mut tw = mush_testkit::TestWorld::with_modules(Vec::new());
    let core = tw.spawn_player("a live stand-in for the core");
    let hull = tw.spawn_player("a live stand-in for the hull");
    let mut snap = snapshot(SELLER);
    snap.put_asset(EconomicAsset {
        object: core,
        ..title(CORE, SELLER, 4)
    });
    snap.put_asset(EconomicAsset {
        object: hull,
        ..title(HULL, SELLER, 6)
    });
    snap.managed_titles =
        crate::managed_title::ManagedTitleIndex::Ready(BTreeMap::from([(core, birth())]));
    let store = crate::EconomyStore {
        title_issuers: std::sync::Arc::new(std::collections::BTreeSet::from([issuer()])),
        snapshot: snap,
        ..crate::EconomyStore::default()
    };
    let ask = |r: &RegistryCoreResale, store: &crate::EconomyStore, buyer: ObjId| {
        super::titles(&config(), &policy(), store, tw.world(), buyer, r).map(|_| ())
    };
    let good = RegistryCoreResale {
        core,
        hull: Some(ResaleHull {
            hull,
            title_revision: 6,
            hold: Some(HOLD),
        }),
        ..request(false)
    };
    check!(
        ask(&good, &store, BUYER).is_ok(),
        "the seller's own cored hull"
    );
    check!(
        ask(
            &RegistryCoreResale {
                hull: None,
                ..good.clone()
            },
            &store,
            BUYER
        )
        .is_ok(),
        "the seller's own stored core"
    );
    check!(
        ask(&good, &store, SELLER).is_err(),
        "the seller buying their own"
    );
    let mut theirs = store.clone();
    theirs.snapshot.put_asset(EconomicAsset {
        object: core,
        ..title(CORE, OTHER, 4)
    });
    check!(ask(&good, &theirs, BUYER).is_err(), "another holder's core");
    let mut claimed = store.clone();
    let mut held = EconomicAsset {
        object: core,
        ..title(CORE, SELLER, 4)
    };
    held.encumbrances
        .push(crate::record::Encumbrance::Disputed {
            claimant: PrincipalId::Organization(OrgId(9)),
            reason: "x".into(),
        });
    claimed.snapshot.put_asset(held);
    check!(ask(&good, &claimed, BUYER).is_err(), "a claimed core");
    let mut hold = store.clone();
    let mut vault = hold.snapshot.vault(HOLD).cloned().ok_or("the hold")?;
    vault.owner = PrincipalId::Object(OTHER);
    hold.snapshot.put_vault(vault);
    check!(ask(&good, &hold, BUYER).is_err(), "a hold not the seller's");
    let mut purse = store.clone();
    purse
        .snapshot
        .put_account(account(PURSE, PrincipalId::Object(OTHER), 9_000_000));
    check!(
        ask(&good, &purse, BUYER).is_err(),
        "a purse not the buyer's"
    );
    let free = RegistryCoreResale {
        price: Amount::ZERO,
        ..good.clone()
    };
    check!(ask(&free, &store, BUYER).is_err(), "a zero price");
    Ok(())
}
