//! Genuine seller offer and named-buyer acceptance used by title-loss fixtures.
//! The exact prepared unions are persisted before owner installation. This
//! models the writer ACK boundary, not server transport or a pending transfer.
use super::*;
use mush_core::{OfferExecutionContext, OfferSubmission};

pub(super) const OFFER: &str = "offer_mind_charter_transfer";
pub(super) const ACCEPT: &str = "accept_mind_charter_transfer";

pub(super) fn signed(
    engine: &mut Engine,
    id: &str,
    actor: ObjId,
    n: u128,
    parameters: Value,
) -> TestResult<PreparedComposedTxn> {
    engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x76; 32])?,
        [],
    )?);
    let exact = engine
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
        .ok_or("actual charter transfer action")?;
    let principal = PrincipalId::Object { id: actor };
    let query = engine.legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&exact)
        .ok_or("actual charter transfer offer")?;
    let request = request(n, actor, "roll", parameters.clone())?;
    let resolved = engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: request.request(),
            principal,
            actor,
            idempotency: request.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: offer.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters,
        },
    )?;
    let (action, invocation, _) = resolved.into_authoritative_parts();
    assert_eq!(action, exact);
    Ok(engine
        .prepare_registered_composed_resolved_authoritative_action(&exact, &invocation, token(n))?
        .0)
}

fn unique_wallet(
    economy: &EconomyStore,
    actor: ObjId,
    instrument: Option<&InstrumentId>,
) -> TestResult<Wallet> {
    let mut matching = economy.snapshot().accounts().filter(|account| {
        account.owner == EconomicOwner::Object(actor)
            && account.scope.is_none()
            && account.status == AccountStatus::Active
            && instrument.is_none_or(|instrument| &account.instrument == instrument)
    });
    let wallet = matching.next().ok_or("actual personal wallet")?.clone();
    if matching.next().is_some() {
        return Err("ambiguous actual personal wallet".into());
    }
    Ok(wallet)
}

pub(super) fn consideration(
    engine: &Engine,
    charter: ObjId,
    buyer: ObjId,
) -> TestResult<(ObjId, Wallet, Wallet)> {
    let economy = engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("actual Economy")?;
    let title = economy.snapshot().asset(charter).ok_or("actual charter")?;
    let EconomicOwner::Object(seller) = title.title_owner else {
        return Err("personal charter required".into());
    };
    let payout = unique_wallet(economy, seller, None)?;
    let payer = unique_wallet(economy, buyer, Some(&payout.instrument))?;
    Ok((seller, payout, payer))
}

pub(super) fn offer_params(charter: ObjId, buyer: ObjId, payout: WalletId) -> Value {
    json!({
        "charter":charter.0.to_string(),
        "buyer":buyer.0.to_string(),
        "seller_account":payout.0.to_string(),
        "price_minor":"1",
        "valid_for_seconds":"600"
    })
}

pub(super) fn sale(
    engine: &mut Engine,
    charter: ObjId,
    buyer: ObjId,
    n: u128,
    mut persist: impl FnMut(&[ReplayWrite]) -> TestResult,
) -> TestResult {
    let (seller, payout, payer) = consideration(engine, charter, buyer)?;
    let before_title = engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("actual Economy")?
        .snapshot()
        .asset(charter)
        .ok_or("actual charter")?
        .clone();
    let offered = signed(
        engine,
        OFFER,
        seller,
        n,
        offer_params(charter, buyer, payout.id),
    )?;
    let (writes, install) = offered.into_parts();
    persist(&writes)?;
    engine.install_composed_action(install)?;
    let accepted = signed(
        engine,
        ACCEPT,
        buyer,
        n.checked_add(1).ok_or("charter transfer token overflow")?,
        json!({"offer":token(n),"payer":payer.id.0.to_string()}),
    )?;
    let (writes, install) = accepted.into_parts();
    persist(&writes)?;
    engine.install_composed_action(install)?;
    let economy = engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("actual Economy")?;
    let title = economy
        .snapshot()
        .asset(charter)
        .ok_or("actual transferred charter")?;
    assert_eq!(title.title_owner, EconomicOwner::Object(buyer));
    assert_eq!(
        title.revision,
        before_title
            .revision
            .checked_add(1)
            .ok_or("title revision")?
    );
    let after_payer = economy.snapshot().account(payer.id).ok_or("actual payer")?;
    let after_payout = economy
        .snapshot()
        .account(payout.id)
        .ok_or("actual payout")?;
    assert_eq!(
        after_payer.posted_balance.minor(),
        payer.posted_balance.minor() - 1
    );
    assert_eq!(
        after_payout.posted_balance.minor(),
        payout.posted_balance.minor() + 1
    );
    assert_eq!(after_payer.held_amount, payer.held_amount);
    assert_eq!(after_payout.held_amount, payout.held_amount);
    Ok(())
}

pub(super) fn module_writes(writes: &[ReplayWrite]) -> TestResult<Vec<ModuleOp>> {
    writes
        .iter()
        .map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => Ok(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: key.to_string().into(),
                    value: value.clone(),
                },
            }),
            ReplayWrite::ModuleDelete { namespace, key } => Ok(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::Del {
                    key: key.to_string().into(),
                },
            }),
            ReplayWrite::LedgerChunkSet { write } => Ok(ModuleOp {
                ns: write.namespace().to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: write.key().to_string().into(),
                    value: serde_json::to_value(write.chunk())?,
                },
            }),
            ReplayWrite::ObjectSet { .. }
            | ReplayWrite::ObjectDelete { .. }
            | ReplayWrite::ObjectAllocatorSet { .. } => {
                Err("charter sale unexpectedly changed a Core object".into())
            }
        })
        .collect()
}
