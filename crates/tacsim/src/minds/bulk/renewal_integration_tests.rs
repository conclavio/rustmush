//! Real D birth -> registered Market/Economy/Roll renewal. Every grant below
//! comes from the actual composed provider and owner installation. This is an
//! in-process ACK/replay fixture, not a server writer/Unknown transport test.
use super::*;
use mush_core::authoritative::ReplayWrite;
use mush_market::mind_renewal::{MindRenewalOperationV1, MindRenewalReceiptV1};
use mush_roll::commerce::{
    TermGeneration,
    renewal::{RenewalAttemptV1, RenewalHeadV1, RenewalOperationV1, RenewalStateV1},
};

#[path = "renewal_service_integration_tests.rs"]
pub(super) mod service_tests;

const START: u128 = 9_000_000;
const DRAFT: &str = "draft_mind_renewal";
const FUND: &str = "fund_mind_renewal";
const GRANT: &str = "settle_mind_renewal";
const REFUND: &str = "refund_mind_renewal";
const CLOSE: &str = "close_mind_renewal";
const ROLL_OP: &str = "roll_mind_renewal_operation";
const HEAD: &str = "roll_mind_renewal_head";
const ECONOMICS: &str = "roll_mind_paid_term_economics";

fn row<T: serde::de::DeserializeOwned>(
    world: &PlayerWorld,
    owner: &str,
    family: &str,
    field: &str,
    exact: Value,
) -> TestResult<T> {
    let mut matched = rows(&disk(world), owner, family)
        .into_iter()
        .filter(|value| value.get(field) == Some(&exact));
    let value = matched.next().ok_or("missing actual renewal row")?;
    assert!(matched.next().is_none(), "renewal key is unique");
    Ok(serde_json::from_value(value)?)
}
fn licence(world: &PlayerWorld, id: LicenceId) -> TestResult<PersonalPaidLicence> {
    row(
        world,
        "roll",
        "roll_mind_personal_paid_licence",
        "id",
        json!(id),
    )
}
fn term(
    world: &PlayerWorld,
    id: LicenceId,
    generation: TermGeneration,
) -> TestResult<PaidTermRecord> {
    let mut found = rows(&disk(world), "roll", "roll_mind_paid_term")
        .into_iter()
        .filter(|v| {
            v.get("licence") == Some(&json!(id)) && v.get("generation") == Some(&json!(generation))
        });
    let actual = found.next().ok_or("actual retained generation")?;
    assert!(found.next().is_none(), "term generation is unique");
    Ok(serde_json::from_value(actual)?)
}
fn economics(
    world: &PlayerWorld,
    id: LicenceId,
    generation: TermGeneration,
) -> TestResult<PaidTermEconomicsV1> {
    let mut found = rows(&disk(world), "roll", ECONOMICS)
        .into_iter()
        .filter(|v| {
            v.get("licence") == Some(&json!(id)) && v.get("generation") == Some(&json!(generation))
        });
    let actual = found.next().ok_or("actual retained economics")?;
    assert!(found.next().is_none(), "economics generation is unique");
    Ok(serde_json::from_value(actual)?)
}
fn operation(world: &PlayerWorld, id: CommitToken) -> TestResult<RenewalOperationV1> {
    row(world, "roll", ROLL_OP, "operation", json!(id))
}
fn market_operation(world: &PlayerWorld, id: CommitToken) -> TestResult<MindRenewalOperationV1> {
    row(
        world,
        "market",
        MindRenewalOperationV1::FAMILY,
        "operation",
        json!(id),
    )
}
fn head(world: &PlayerWorld, id: LicenceId) -> TestResult<RenewalHeadV1> {
    row(world, "roll", HEAD, "licence", json!(id))
}
fn paired(world: &PlayerWorld, id: CommitToken) -> TestResult<RenewalOperationV1> {
    let actual = operation(world, id)?;
    let market = market_operation(world, id)?;
    market.matches_roll(&actual)?;
    let economy = world
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("real Economy")?;
    let audit = economy.renewal_audit()?;
    let proof = audit.get(id)?;
    actual.verify_history(&proof)?;
    market.verify_history(&proof)?;
    let receipts = rows(&disk(world), "market", MindRenewalReceiptV1::FAMILY);
    let receipts: Vec<_> = receipts
        .iter()
        .filter(|v| v.get("operation") == Some(&json!(id)))
        .collect();
    if actual.state().pending() {
        assert!(receipts.is_empty(), "pending offer has no terminal receipt");
    } else {
        assert_eq!(receipts.len(), 1);
        let receipt: MindRenewalReceiptV1 =
            serde_json::from_value((*receipts.first().ok_or("receipt")?).clone())?;
        receipt.verify_history(&market, &proof)?;
    }
    Ok(actual)
}
fn prepare_step(
    world: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    number: u128,
    params: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    super::prepare(world, "market", id, 1, actor, number, params)
}
fn step(world: &mut PlayerWorld, id: &str, number: u128, params: Value) -> TestResult {
    super::commit(world, "market", id, 1, GOD, number, params)
}
fn step_for(world: &mut PlayerWorld, id: &str, number: u128, operation: CommitToken) -> TestResult {
    step(world, id, number, json!({"operation":operation}))
}
fn assert_disk(world: &PlayerWorld, before: &Disk) {
    assert!(
        disk(world) == *before,
        "pre-ACK or refused action changed retained images"
    );
}
fn typed_families(writes: &[ReplayWrite]) -> Vec<String> {
    let mut out: Vec<_> = writes
        .iter()
        .filter_map(|write| match write {
            ReplayWrite::ModuleJsonSet { value, .. } => value
                .pointer("/family/id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        })
        .collect();
    out.sort();
    out
}
fn assert_current_unchanged(
    world: &PlayerWorld,
    old: &PersonalPaidLicence,
    saved_term: &PaidTermRecord,
    saved_economics: &PaidTermEconomicsV1,
) -> TestResult {
    assert_eq!(licence(world, old.id())?, *old);
    assert_eq!(term(world, old.id(), old.term())?, *saved_term);
    assert_eq!(economics(world, old.id(), old.term())?, *saved_economics);
    Ok(())
}
fn due_world(
    f: &PurchaseFixture,
    saved: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult<PlayerWorld> {
    let mut world = PlayerWorld::boot_renewals(f, saved)?;
    let current = licence(&world, birth.licence())?;
    let predecessor = term(&world, current.id(), current.term())?;
    let due = predecessor.window().term_end().get();
    world
        .engine
        .world
        .set_now(world.engine.world.now().max(due));
    Ok(world)
}
#[inline(never)]
fn check_early_renewal(
    world: &mut PlayerWorld,
    birth: &EntitlementAllocationV1,
    old_term: &PaidTermRecord,
) -> TestResult {
    let before = disk(world);
    assert!(world.engine.world.now() < old_term.window().term_end().get());
    assert!(
        prepare_step(
            world,
            DRAFT,
            GOD,
            START + 89,
            json!({"licence":birth.licence().get()})
        )
        .is_err(),
        "early renewal cannot truncate a paid term"
    );
    world.engine.world.set_now(
        old_term
            .window()
            .term_end()
            .get()
            .checked_sub(1)
            .ok_or("due boundary")?,
    );
    assert!(
        prepare_step(
            world,
            DRAFT,
            GOD,
            START + 89,
            json!({"licence":birth.licence().get()})
        )
        .is_err(),
        "one second before term_end remains too early"
    );
    assert_disk(world, &before);
    world
        .engine
        .world
        .set_now(old_term.window().term_end().get());
    Ok(())
}

fn grant_chain(
    f: &PurchaseFixture,
    granted: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult<(Durable, Durable)> {
    let mut world = PlayerWorld::boot_renewals(f, granted)?;
    let old = licence(&world, birth.licence())?;
    let old_term = term(&world, old.id(), old.term())?;
    let old_economics = economics(&world, old.id(), old.term())?;
    check_early_renewal(&mut world, birth, &old_term)?;
    let old_balances = balances(&world)?;
    let before = disk(&world);
    let params = json!({"licence":birth.licence().get()});
    // No host-supplied price/payee/form and no other character's request.
    for (actor, bad) in [
        (BUYER, params.clone()),
        (GOD, json!({"licence":birth.licence().get(),"price":1})),
        (
            GOD,
            json!({"licence":birth.licence().get(),"previous_operation":token(START)}),
        ),
    ] {
        assert!(prepare_step(&mut world, DRAFT, actor, START + 90, bad).is_err());
        assert_disk(&world, &before);
    }
    let (prepared, _) = prepare_step(&mut world, DRAFT, GOD, START, params)?;
    assert_disk(&world, &before);
    assert_eq!(balances(&world)?, old_balances);
    assert_current_unchanged(&world, &old, &old_term, &old_economics)?;
    let (writes, install) = prepared.into_parts();
    apply(&mut world, &writes, install)?;
    let draft = paired(&world, token(START))?;
    assert!(matches!(draft.state(), RenewalStateV1::Drafted {}));
    assert_eq!(draft.attempt(), RenewalAttemptV1::Automatic {});
    assert_eq!(draft.binding().birth_operation(), birth.operation());
    assert_eq!(draft.binding().predecessor(), old.term());
    assert_eq!(head(&world, old.id())?.pending(), Some(token(START)));
    assert_eq!(balances(&world)?, old_balances);
    assert_current_unchanged(&world, &old, &old_term, &old_economics)?;
    source_answer(&mut world, GOD, old.id(), Some(birth.revision().source()))?;
    let drafted = capture(&world, &disk(&world));
    // Pending automatic/retry overlap must not replace the current head.
    for params in [
        json!({"licence":old.id().get()}),
        json!({"previous_operation":token(START)}),
    ] {
        let before = disk(&world);
        assert!(prepare_step(&mut world, DRAFT, GOD, START + 91, params).is_err());
        assert_disk(&world, &before);
    }
    let before = disk(&world);
    let (prepared, _) = prepare_step(
        &mut world,
        FUND,
        GOD,
        START + 1,
        json!({"operation":token(START)}),
    )?;
    assert_disk(&world, &before);
    assert_eq!(balances(&world)?, old_balances);
    let (writes, install) = prepared.into_parts();
    assert_eq!(
        typed_families(&writes),
        vec![
            MindRenewalOperationV1::FAMILY.to_owned(),
            ROLL_OP.to_owned()
        ]
    );
    apply(&mut world, &writes, install)?;
    let funded = paired(&world, token(START))?;
    assert!(matches!(funded.state(), RenewalStateV1::Funded { .. }));
    assert_current_unchanged(&world, &old, &old_term, &old_economics)?;
    source_answer(&mut world, GOD, old.id(), Some(birth.revision().source()))?;
    for (id, posted, held) in &old_balances {
        let all = balances(&world)?;
        let actual = all.iter().find(|v| v.0 == *id).ok_or("wallet")?;
        assert_eq!(actual.1, *posted);
        assert_eq!(
            actual.2,
            held.checked_add(Amount::from_minor(if id.0 == 0 { 73 } else { 0 }))
                .ok_or("held")?
        );
    }
    let funded_image = capture(&world, &disk(&world));
    // Cold funded recovery keeps the old pointer and escrow; no synthetic grant.
    let mut cold = PlayerWorld::boot_renewals(f, &funded_image)?;
    paired(&cold, token(START))?;
    assert_current_unchanged(&cold, &old, &old_term, &old_economics)?;
    cold.engine
        .world
        .set_now(cold.engine.world.now().checked_add(31).ok_or("time")?);
    let before = disk(&cold);
    let held = balances(&cold)?;
    let (prepared, _) = prepare_step(
        &mut cold,
        GRANT,
        GOD,
        START + 2,
        json!({"operation":token(START)}),
    )?;
    assert_disk(&cold, &before);
    assert_eq!(balances(&cold)?, held);
    let (writes, install) = prepared.into_parts();
    apply(&mut cold, &writes, install)?;
    let renewed = paired(&cold, token(START))?;
    assert!(renewed.state().granted());
    let current = licence(&cold, old.id())?;
    assert_eq!(current.operation(), birth.operation());
    assert_eq!(current.term().get(), old.term().get() + 1);
    let new_term = term(&cold, current.id(), current.term())?;
    let new_economics = economics(&cold, current.id(), current.term())?;
    assert_eq!(new_term.operation(), token(START));
    assert_eq!(new_economics.operation(), token(START));
    assert_eq!(new_economics.grant(), old_economics.grant());
    assert_eq!(
        new_economics.product_digest(),
        old_economics.product_digest()
    );
    assert_eq!(term(&cold, old.id(), old.term())?, old_term);
    assert_eq!(economics(&cold, old.id(), old.term())?, old_economics);
    assert_eq!(new_term.window().issued_at().get(), cold.engine.world.now());
    assert_eq!(head(&cold, old.id())?.current(), current.term());
    assert_eq!(head(&cold, old.id())?.pending(), None);
    for (id, posted, held) in &old_balances {
        let all = balances(&cold)?;
        let actual = all.iter().find(|v| v.0 == *id).ok_or("wallet")?;
        let delta = match id.0 {
            0 => -73,
            1 => 7,
            2 => 66,
            _ => 0,
        };
        assert_eq!(
            actual.1,
            posted
                .checked_add(Amount::from_minor(delta))
                .ok_or("payout")?
        );
        assert_eq!(actual.2, *held);
    }
    let snapshot = capture(&cold, &disk(&cold));
    let replay = PlayerWorld::boot_renewals(f, &snapshot)?;
    paired(&replay, token(START))?;
    assert_eq!(licence(&replay, old.id())?, current);
    assert_eq!(balances(&replay)?, balances(&cold)?);
    assert!(
        PlayerWorld::boot_entitlements(f, &snapshot).is_err(),
        "disabled renewal composition rejects retained history"
    );
    Ok((snapshot, drafted))
}

fn retries(f: &PurchaseFixture, granted: &Durable, birth: &EntitlementAllocationV1) -> TestResult {
    for (index, ending) in ["declined", "expired", "refund"].into_iter().enumerate() {
        let number = START + 100 + u128::try_from(index)? * 10;
        let mut world = due_world(f, granted, birth)?;
        let old = licence(&world, birth.licence())?;
        let old_term = term(&world, old.id(), old.term())?;
        let old_economics = economics(&world, old.id(), old.term())?;
        let initial = balances(&world)?;
        step(&mut world, DRAFT, number, json!({"licence":old.id().get()}))?;
        if ending == "refund" {
            step_for(&mut world, FUND, number + 1, token(number))?;
            step_for(&mut world, REFUND, number + 2, token(number))?;
        } else {
            if ending == "expired" {
                let expires = market_operation(&world, token(number))?
                    .offer()
                    .terms
                    .expires_at
                    .at;
                assert!(
                    prepare_step(
                        &mut world,
                        CLOSE,
                        GOD,
                        number + 2,
                        json!({"operation":token(number),"reason":"expired"})
                    )
                    .is_err()
                );
                world.engine.world.set_now(expires);
            }
            step(
                &mut world,
                CLOSE,
                number + 2,
                json!({"operation":token(number),"reason":ending}),
            )?;
        }
        let closed = paired(&world, token(number))?;
        assert!(closed.state().retryable());
        assert_eq!(balances(&world)?, initial);
        assert_current_unchanged(&world, &old, &old_term, &old_economics)?;
        assert!(term(&world, old.id(), closed.binding().next()).is_err());
        assert!(economics(&world, old.id(), closed.binding().next()).is_err());
        assert_eq!(head(&world, old.id())?.pending(), None);
        // Terminal history is durable before any retry and grants no extension.
        let closed_snapshot = capture(&world, &disk(&world));
        let mut world = PlayerWorld::boot_renewals(f, &closed_snapshot)?;
        paired(&world, token(number))?;
        let before = disk(&world);
        assert!(
            prepare_step(
                &mut world,
                DRAFT,
                GOD,
                number + 3,
                json!({"licence":old.id().get()})
            )
            .is_err(),
            "same predecessor requires explicit new consent/retry link"
        );
        assert!(
            prepare_step(
                &mut world,
                DRAFT,
                GOD,
                number + 3,
                json!({"previous_operation":token(number+99)})
            )
            .is_err()
        );
        assert_disk(&world, &before);
        step(
            &mut world,
            DRAFT,
            number + 3,
            json!({"previous_operation":token(number)}),
        )?;
        let retry = paired(&world, token(number + 3))?;
        assert_eq!(
            retry.attempt(),
            RenewalAttemptV1::BuyerRetry {
                previous_operation: token(number)
            }
        );
        assert_ne!(retry.contract(), closed.contract());
        assert_eq!(retry.binding(), closed.binding());
        assert_ne!(retry.consent_hash(), closed.consent_hash());
        step_for(&mut world, FUND, number + 4, token(number + 3))?;
        step_for(&mut world, GRANT, number + 5, token(number + 3))?;
        paired(&world, token(number))?;
        paired(&world, token(number + 3))?;
        assert_eq!(
            term(&world, old.id(), retry.binding().next())?.operation(),
            token(number + 3)
        );
        assert_eq!(
            economics(&world, old.id(), retry.binding().next())?.operation(),
            token(number + 3)
        );
        assert_eq!(licence(&world, old.id())?.operation(), birth.operation());
        // Old Closed/Refunded rows coexist lawfully with the linked retry's
        // successful next generation. No row is attributed to the old attempt.
        let complete = capture(&world, &disk(&world));
        let cold = PlayerWorld::boot_renewals(f, &complete)?;
        paired(&cold, token(number))?;
        paired(&cold, token(number + 3))?;
        assert_eq!(balances(&cold)?, balances(&world)?);
        assert_eq!(head(&cold, old.id())?.last_operation(), token(number + 3));
    }
    Ok(())
}

fn title_fences(
    f: &PurchaseFixture,
    granted: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    for (offset, id) in [(0, DRAFT), (10, FUND), (20, GRANT)] {
        for after_stage in [false, true] {
            let number = START + 200 + offset;
            let mut world = due_world(f, granted, birth)?;
            if id != DRAFT {
                step(
                    &mut world,
                    DRAFT,
                    number,
                    json!({"licence":birth.licence().get()}),
                )?;
            }
            if id == GRANT {
                step_for(&mut world, FUND, number + 1, token(number))?;
            }
            let params = if id == DRAFT {
                json!({"licence":birth.licence().get()})
            } else {
                json!({"operation":token(number)})
            };
            let definition = action(&world, "market", id, 1)?;
            let number_for_action = number + 2;
            let mut before = balances(&world)?;
            let old = licence(&world, birth.licence())?;
            let (prepared, output) =
                prepare_step(&mut world, id, GOD, number_for_action, params.clone())?;
            if after_stage {
                stage(
                    &mut world,
                    definition.clone(),
                    GOD,
                    number_for_action,
                    params.clone(),
                    prepared,
                    output,
                )?;
                world.transfer_charter(birth.charter().object(), GOD, 49_000_130)?;
                before = balances(&world)?;
                assert!(world.engine.take_staged_action().is_err());
            } else {
                world.transfer_charter(birth.charter().object(), GOD, 49_000_140)?;
                before = balances(&world)?;
                assert!(
                    stage(
                        &mut world,
                        definition,
                        GOD,
                        number_for_action,
                        params,
                        prepared,
                        output
                    )
                    .is_err()
                );
            }
            assert_eq!(
                world
                    .engine
                    .world
                    .module_state::<EconomyStore>()
                    .ok_or("Economy")?
                    .snapshot()
                    .asset(ObjId(birth.charter().get()))
                    .ok_or("title")?
                    .title_owner,
                EconomicOwner::Object(GOD)
            );
            assert_eq!(licence(&world, birth.licence())?, old);
            assert_eq!(balances(&world)?, before);
            assert!(rows(&disk(&world), "market", MindRenewalReceiptV1::FAMILY).is_empty());
            if id == DRAFT {
                assert!(rows(&disk(&world), "roll", ROLL_OP).is_empty());
            } else {
                assert!(paired(&world, token(number))?.state().pending());
            }
        }
    }
    Ok(())
}
fn accounts_and_expiry(
    f: &PurchaseFixture,
    granted: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    {
        let mut world = due_world(f, granted, birth)?;
        let old = licence(&world, birth.licence())?;
        let due = term(&world, old.id(), old.term())?
            .window()
            .term_end()
            .get();
        let before = disk(&world);
        let money = balances(&world)?;
        let params = json!({"licence":birth.licence().get()});
        let definition = action(&world, "market", DRAFT, 1)?;
        let (prepared, output) = prepare_step(&mut world, DRAFT, GOD, START + 249, params.clone())?;
        stage(
            &mut world,
            definition,
            GOD,
            START + 249,
            params,
            prepared,
            output,
        )?;
        world
            .engine
            .world
            .set_now(due.checked_sub(1).ok_or("due boundary")?);
        assert!(
            world.engine.take_staged_action().is_err(),
            "captured due eligibility cannot publish before predecessor term_end"
        );
        assert_disk(&world, &before);
        assert_eq!(balances(&world)?, money);
    }
    for (offset, id) in [(0, DRAFT), (10, FUND), (20, GRANT)] {
        let number = START + 250 + offset;
        let mut world = due_world(f, granted, birth)?;
        if id != DRAFT {
            step(
                &mut world,
                DRAFT,
                number,
                json!({"licence":birth.licence().get()}),
            )?;
        }
        if id == GRANT {
            step_for(&mut world, FUND, number + 1, token(number))?;
        }
        let params = if id == DRAFT {
            json!({"licence":birth.licence().get()})
        } else {
            json!({"operation":token(number)})
        };
        let definition = action(&world, "market", id, 1)?;
        let before = disk(&world);
        let money = balances(&world)?;
        let (prepared, output) = prepare_step(&mut world, id, GOD, number + 2, params.clone())?;
        let seller = merge_login_accounts(f)?;
        let refusal = stage(
            &mut world,
            definition,
            GOD,
            number + 2,
            params,
            prepared,
            output,
        )
        .is_err();
        restore_login_account(f, seller)?;
        assert!(
            refusal,
            "retained actual login account pair must be rechecked"
        );
        assert_disk(&world, &before);
        assert_eq!(balances(&world)?, money);
    }
    for (offset, id) in [(0, FUND), (10, GRANT)] {
        let number = START + 300 + offset;
        let mut world = due_world(f, granted, birth)?;
        step(
            &mut world,
            DRAFT,
            number,
            json!({"licence":birth.licence().get()}),
        )?;
        if id == GRANT {
            step_for(&mut world, FUND, number + 1, token(number))?;
        }
        let expires = market_operation(&world, token(number))?
            .offer()
            .terms
            .expires_at
            .at;
        let definition = action(&world, "market", id, 1)?;
        let params = json!({"operation":token(number)});
        let before = disk(&world);
        let money = balances(&world)?;
        let (prepared, output) = prepare_step(&mut world, id, GOD, number + 2, params.clone())?;
        stage(
            &mut world,
            definition,
            GOD,
            number + 2,
            params,
            prepared,
            output,
        )?;
        world.engine.world.set_now(expires);
        assert!(
            world.engine.take_staged_action().is_err(),
            "exact offer expiry at custody refuses fresh money/grant"
        );
        assert_disk(&world, &before);
        assert_eq!(balances(&world)?, money);
    }
    Ok(())
}

fn cleanup_after_parent_lapse(
    f: &PurchaseFixture,
    granted: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    let parent: PaidTermRecord = serde_json::from_value(
        rows(&granted.disk, "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|v| v.get("licence") != Some(&json!(birth.licence())))
            .ok_or("actual issuer parent")?,
    )?;
    for funded in [false, true] {
        let number = START + 350 + u128::from(funded) * 10;
        let mut world = due_world(f, granted, birth)?;
        let old = licence(&world, birth.licence())?;
        let own = term(&world, old.id(), old.term())?;
        let money = balances(&world)?;
        step(
            &mut world,
            DRAFT,
            number,
            json!({"licence":birth.licence().get()}),
        )?;
        if funded {
            step_for(&mut world, FUND, number + 1, token(number))?;
        }
        world
            .engine
            .world
            .set_now(parent.window().grace_end().get());
        assert_eq!(
            own.window()
                .phase_at(mush_roll::commerce::WorldIcSeconds::from_stamp(
                    &world.engine.world.world_ic_now()
                )?),
            mush_roll::commerce::TermPhase::Grace
        );
        source_answer(
            &mut world,
            GOD,
            birth.licence(),
            Some(birth.revision().source()),
        )?;
        let lapsed = capture(&world, &disk(&world));
        let mut cold = PlayerWorld::boot_renewals(f, &lapsed)?;
        // Fresh work is unavailable, while captured custody can still close.
        // Both offer expiry and issuer lapse are present in this long-gap case.
        assert!(
            prepare_step(
                &mut cold,
                DRAFT,
                GOD,
                number + 2,
                json!({"licence":birth.licence().get()})
            )
            .is_err()
        );
        assert!(
            prepare_step(
                &mut cold,
                if funded { GRANT } else { FUND },
                GOD,
                number + 2,
                json!({"operation":token(number)})
            )
            .is_err()
        );
        if funded {
            step_for(&mut cold, REFUND, number + 3, token(number))?;
        } else {
            step(
                &mut cold,
                CLOSE,
                number + 3,
                json!({"operation":token(number),"reason":"expired"}),
            )?;
        }
        paired(&cold, token(number))?;
        assert_eq!(balances(&cold)?, money);
        assert_eq!(licence(&cold, old.id())?, old);
        assert!(
            prepare_step(
                &mut cold,
                DRAFT,
                GOD,
                number + 4,
                json!({"previous_operation":token(number)})
            )
            .is_err(),
            "retry requires fresh issuer eligibility"
        );
        let ended = capture(&cold, &disk(&cold));
        let replay = PlayerWorld::boot_renewals(f, &ended)?;
        paired(&replay, token(number))?;
    }
    Ok(())
}

fn corruptions(
    f: &PurchaseFixture,
    snapshot: &Durable,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    for (owner, family) in [
        ("market", MindRenewalOperationV1::FAMILY),
        ("market", MindRenewalReceiptV1::FAMILY),
        ("roll", ROLL_OP),
        ("roll", HEAD),
        ("roll", ECONOMICS),
    ] {
        let mut lost = snapshot.clone();
        let count = lost.disk.len();
        lost.disk
            .retain(|(ns, _), v| !(ns == owner && v.pointer("/family/id") == Some(&json!(family))));
        assert!(lost.disk.len() < count, "removed actual persisted family");
        assert!(
            PlayerWorld::boot_renewals(f, &lost).is_err(),
            "missing or all-missing {family} must refuse"
        );
    }
    // One side absent while E4/roots/head survive cannot be mistaken for zero jobs.
    let mut lost = snapshot.clone();
    let count = lost.disk.len();
    lost.disk.retain(|_, v| {
        ![
            ROLL_OP,
            HEAD,
            MindRenewalOperationV1::FAMILY,
            MindRenewalReceiptV1::FAMILY,
        ]
        .iter()
        .any(|name| v.pointer("/family/id") == Some(&json!(name)))
    });
    assert!(lost.disk.len() < count);
    assert!(PlayerWorld::boot_renewals(f, &lost).is_err());
    let mut missing_economy = snapshot.clone();
    let before = missing_economy.disk.len();
    missing_economy
        .disk
        .retain(|(ns, key), _| !(ns == "economy" && key.starts_with("renewal-offer:")));
    assert!(
        missing_economy.disk.len() < before,
        "removed the actual E4 offer history"
    );
    assert!(
        PlayerWorld::boot_renewals(f, &missing_economy).is_err(),
        "receipt/contract alone cannot replace missing E4 origins"
    );
    let mut missing_current_term = snapshot.clone();
    let before = missing_current_term.disk.len();
    missing_current_term.disk.retain(|(ns, _), value| {
        !(ns == "roll"
            && value.pointer("/family/id") == Some(&json!("roll_mind_paid_term"))
            && value.pointer("/value/licence") == Some(&json!(birth.licence()))
            && value.pointer("/value/generation") == Some(&json!(2)))
    });
    assert!(missing_current_term.disk.len() < before);
    assert!(
        PlayerWorld::boot_renewals(f, &missing_current_term).is_err(),
        "current pointer cannot substitute for its term"
    );
    // Exact current pointer and retained immutable birth are independently joined.
    for field in ["term", "operation"] {
        let mut crossed = snapshot.clone();
        let mut changed = false;
        for ((ns, _), v) in &mut crossed.disk {
            if ns == "roll"
                && v.pointer("/family/id") == Some(&json!("roll_mind_personal_paid_licence"))
                && v.pointer("/value/id") == Some(&json!(birth.licence()))
            {
                let value = v
                    .get_mut("value")
                    .and_then(Value::as_object_mut)
                    .ok_or("licence document")?;
                let slot = value.get_mut(field).ok_or("licence field")?;
                *slot = if field == "term" {
                    json!(1)
                } else {
                    json!(token(START))
                };
                changed = true;
            }
        }
        assert!(changed);
        assert!(
            PlayerWorld::boot_renewals(f, &crossed).is_err(),
            "crossed {field}"
        );
    }
    Ok(())
}

async fn customer(
    f: &PurchaseFixture,
    granted: &Durable,
    owner: LicenceId,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let parent: PaidTermRecord = serde_json::from_value(
        rows(&granted.disk, "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|r| r.get("licence") != Some(&json!(a.licence())))
            .ok_or("parent")?,
    )?;
    let mut lapsed = granted.clone();
    lapsed.now = parent.window().grace_end().get();
    let mut active = PlayerWorld::boot_renewals(f, &lapsed)?;
    let current = licence(&active, a.licence())?;
    let own = term(&active, current.id(), current.term())?;
    assert_eq!(
        own.window()
            .phase_at(mush_roll::commerce::WorldIcSeconds::from_stamp(
                &active.engine.world.world_ic_now()
            )?),
        mush_roll::commerce::TermPhase::Active
    );
    source_answer(&mut active, GOD, a.licence(), Some(a.revision().source()))?;
    lapsed.now = own.window().term_end().get();
    assert!(lapsed.now >= parent.window().grace_end().get());
    drop(active);
    let mut world = PlayerWorld::boot_renewals(f, &lapsed)?;
    let tasks = world.tasks.clone();
    let result:TestResult=async {
        let current=licence(&world,a.licence())?;
        assert!(current.term().get()>1);
        let own=term(&world,a.licence(),current.term())?;
        assert_eq!(own.window().phase_at(mush_roll::commerce::WorldIcSeconds::from_stamp(&world.engine.world.world_ic_now())?),mush_roll::commerce::TermPhase::Grace);
        source_answer(&mut world,GOD,a.licence(),Some(a.revision().source()))?;
        assert!(prepare_step(&mut world,DRAFT,GOD,START+500,json!({"licence":a.licence().get()})).is_err(),"new renewal still needs current issuer parent");
        assert!(super::prepare(&mut world,"market","list_derived_mind",1,BUYER,START+501,listing_params(owner,"open_resale",100)).is_err(),"new sale still needs current issuer parent");
        source_answer(&mut world,BUYER,a.licence(),None)?;
        source_answer(&mut world,BUYER,owner,None)?;
        assert!(rows(&disk(&world),"roll","roll_mind_personal_paid_licence").iter().filter_map(|r|serde_json::from_value::<PersonalPaidLicence>(r.clone()).ok())
            .all(|licence|licence.holder()!=GOD || licence.id()==a.licence()),"customer bought no ancestor licence");
        // New sortie under the customer's own Grace still works after the
        // issuer's selected parent has ended; no ancestor is re-resolved.
        world.say(GOD,"SIM TEAM red")?;world.say(GOD,"SIM AI 1")?;world.say(GOD,"SIM UNIT heavy_mk1")?;
        let selected=world.say(GOD,&format!("SIM MIND filed:{} PROFILE Default",a.licence().get()))?;
        assert!(PlayerWorld::private_text(&selected,GOD).contains("Your slot will carry"));
        world.say(GOD,"SIM START")?;assert_eq!(world.state()?,match_::MatchState::Boarding);
        // The lead character is read during Boarding (`lead_character_of`).
        assert_eq!(match_::lobby::lead_character_of(&world.engine.world,crate::arena::Team::Red),Some(GOD));
        let room=match_::centre_object(&world.engine.world,match_::RED_ROOM_ENTRY).ok_or("red room")?;
        world.engine.world.move_to(GOD,room)?;world.say(GOD,"BOARD")?;assert_eq!(world.state()?,match_::MatchState::Running);
        let id=match_::centre_of(&world.engine.world).and_then(|c|c.match_id).ok_or("match")?;
        let record=world.record(id)?;let unit=record.teams.iter().flat_map(|t|t.pilots.iter()).find_map(|(p,u)|(*p==GOD).then_some(*u)).ok_or("customer unit")?;
        let seal=record.minds.get(&unit).ok_or("actual customer seal")?.seal.clone();
        let certificate=seal.requested.origin.certificate().ok_or("filed certificate")?;
        assert_eq!(certificate.rating_job,a.rating().job());assert_eq!(certificate.rating_id,a.rating().id().digest());
        assert_eq!(certificate.rating_digest,a.rating().receipt_digest());
        let deadline=Instant::now()+WAIT;while !world.snapshots.contains_key(&id){world.pump()?;if Instant::now()>=deadline{return Err("opening snapshot".into());}tokio::time::sleep(Duration::from_millis(10)).await;}
        let opening=world.snapshots.get(&id).ok_or("opening")?;
        let ground=crate::arena_bridge::ground_of(&world.engine.world,id).ok_or("ground")?;
        let arena=crate::arena::Snapshot::decode(&opening.document,(*ground).clone())?;
        assert_eq!(arena.units.iter().find(|u|u.id==unit).ok_or("pilot")?.controller,crate::arena::Controller::Pilot);
        // This causal public EXIT occurs only after own expiry. Future thinks
        // therefore cannot be a cached pre-expiry snapshot masquerading as use.
        world.engine.world.set_now(own.window().grace_end().get());source_answer(&mut world,GOD,a.licence(),None)?;
        let anchor=world.snapshots.get(&id).map_or(-1,|s|s.at_ms);world.say(GOD,"EXIT")?;
        let first=world.progressed_snapshot(id,unit,anchor,0,-1).await?;
        let (thinks,landed)=crate::minds::bulk::registry_integration_tests::original_purchase::paid_checkout::assert_captured(&world,id,unit,&seal,&first)?;
        let second=world.progressed_snapshot(id,unit,first.at_ms,thinks,landed).await?;
        crate::minds::bulk::registry_integration_tests::original_purchase::paid_checkout::assert_captured(&world,id,unit,&seal,&second)?;
        let mut ended=granted.clone();ended.now=own.window().grace_end().get();
        let mut fresh=PlayerWorld::boot_renewals(f,&ended)?;source_answer(&mut fresh,GOD,a.licence(),None)?;
        fresh.say(GOD,"SIM TEAM red")?;let refused=fresh.say(GOD,&format!("SIM MIND filed:{} PROFILE Default",a.licence().get()))?;
        assert!(!PlayerWorld::private_text(&refused,GOD).contains("Your slot will carry"));
        assert!(match_::centre_of(&fresh.engine.world).is_some_and(|c|c.match_id.is_none()));
        fresh.tasks.shutdown(Duration::from_secs(5)).await;Ok(())
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}

pub(super) async fn exercise(
    f: &PurchaseFixture,
    granted: &Durable,
    owner: LicenceId,
    birth: &EntitlementAllocationV1,
) -> TestResult {
    eprintln!("M7_STORY phase=renewal.transitions event=begin");
    service_tests::exercise(f, granted, birth)?;
    let (renewed, _drafted) = grant_chain(f, granted, birth)?;
    retries(f, granted, birth)?;
    title_fences(f, granted, birth)?;
    accounts_and_expiry(f, granted, birth)?;
    cleanup_after_parent_lapse(f, granted, birth)?;
    corruptions(f, &renewed, birth)?;
    story_phase("renewal.customer", || customer(f, &renewed, owner, birth))?.await?;
    eprintln!("M7_STORY phase=renewal.transitions event=complete");
    Ok(())
}
