//! Actual registered owner preparations and custody boundaries. Signed entry and
//! complete ACK installation are exercised by the adjacent genuine sale child.
use super::*;
use mush_core::action::PreparedAuthoritativeAction;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};

fn action(f: &PurchaseFixture, id: &str) -> TestResult<DefinitionRef> {
    f.engine
        .registry
        .definitions()
        .ok_or("actual definitions")?
        .iter()
        .map(|entry| entry.reference())
        .find(|r| {
            r.key().owner.as_str() == "roll"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == 1
        })
        .cloned()
        .ok_or_else(|| "actual charter action".into())
}
fn prepare(
    f: &mut PurchaseFixture,
    action: &DefinitionRef,
    actor: ObjId,
    n: u128,
    p: Value,
) -> TestResult<(PreparedComposedTxn, PreparedAuthoritativeAction)> {
    Ok(f.engine.prepare_registered_composed_authoritative_action(
        action,
        &request(n, actor, "roll", p)?,
        token(n),
    )?)
}
fn stage(
    f: &mut PurchaseFixture,
    action: DefinitionRef,
    actor: ObjId,
    n: u128,
    parameters: Value,
    p: PreparedComposedTxn,
    o: PreparedAuthoritativeAction,
) -> TestResult {
    let inv = request(n, actor, "roll", parameters)?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        o.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(n),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([65; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([66; 32]),
        action,
        provider: OwnerId::new("roll")?,
        cause: mush_core::action::PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: p.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(n))),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?;
    let guards = f
        .engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())?
        .finish()?;
    let _ = f.engine.world.take_dirty();
    f.engine.stage_authoritative_action(p, guards, bundle)?;
    Ok(())
}
fn change(f: &mut PurchaseFixture, mode: u8) -> TestResult {
    match mode {
        0 => f
            .engine
            .world
            .set_now(f.engine.world.now().checked_sub(1).ok_or("UTC")?),
        1 => {
            f.engine
                .world
                .get_mut(BUYER)
                .ok_or("actual buyer")?
                .flags
                .insert(mush_core::Flags::GOING);
        }
        _ => {
            f.engine.registry.freeze_mechanics_owner(
                &OwnerId::new("economy")?,
                mush_core::mechanics::IncidentId(89),
                mush_core::mechanics::RecoveryReason::IntegrityViolation {
                    code: mush_core::mechanics::CanonicalId::new("charter_fixture")?,
                },
            )?;
        }
    }
    let _ = f.engine.world.take_dirty();
    Ok(())
}
pub(super) fn exercise(base: &PurchaseFixture, saved: &Durable) -> TestResult {
    const N: u128 = 48_400_000;
    let mut seed = base.reboot(saved)?;
    let holding = paid_holdings::own(&mut seed, GOD, 1)?;
    let charter = holding.charter().object();
    let (_, payout, payer) = charter_transfer_fixture::consideration(&seed.engine, charter, BUYER)?;
    let params = charter_transfer_fixture::offer_params(charter, BUYER, payout.id);
    let offer = charter_transfer_fixture::signed(
        &mut seed.engine,
        charter_transfer_fixture::OFFER,
        GOD,
        N,
        params.clone(),
    )?;
    seed.install(offer)?;
    let offered = seed.snapshot();
    for accepted in [false, true] {
        for mode in 0..3 {
            for boundary in 0..3 {
                let mut f = base.reboot(if accepted { &offered } else { saved })?;
                let actor = if accepted { BUYER } else { GOD };
                let id = if accepted {
                    charter_transfer_fixture::ACCEPT
                } else {
                    charter_transfer_fixture::OFFER
                };
                let p = if accepted {
                    json!({"offer":token(N),"payer":payer.id.0.to_string()})
                } else {
                    params.clone()
                };
                let exact = action(&f, id)?;
                let before = f.disk.clone();
                let money = f.balances()?;
                if boundary == 0 {
                    change(&mut f, mode)?;
                    // For a fresh seller offer, a lower current UTC has no
                    // earlier retained offer to violate. Test rollback only
                    // after actual preparation; the buyer has a retained bound.
                    if !accepted && mode == 0 {
                        continue;
                    }
                    check!(prepare(&mut f, &exact, actor, N + 1, p).is_err());
                } else {
                    let (prepared, output) = prepare(&mut f, &exact, actor, N + 1, p.clone())?;
                    if boundary == 1 {
                        change(&mut f, mode)?;
                        check!(stage(&mut f, exact, actor, N + 1, p, prepared, output).is_err());
                    } else {
                        stage(&mut f, exact, actor, N + 1, p, prepared, output)?;
                        change(&mut f, mode)?;
                        check!(f.engine.take_staged_action().is_err());
                    }
                }
                check!(
                    f.disk == before,
                    "refused charter operation changed durable images"
                );
                check_eq!(f.balances()?, money);
            }
        }
    }
    Ok(())
}
