//! Genuine acquired OpenResale -> authored saved/Registry certificate ->
//! dependent filing -> new six-target paid commerce. No grant/certificate is
//! constructed. This models owner/core ACK installation, not server transport.
#[path = "minds_facts_integration_tests.rs"]
mod minds_facts;
#[path = "registry_recall_integration_tests.rs"]
mod registry_recall;
#[path = "registry_revocation_integration_tests.rs"]
mod registry_revocation;
#[path = "author_withdrawal_integration_tests.rs"]
mod withdrawal;
use super::dependent_owner_integration_tests::apply;
use super::*;
use crate::minds::bulk::registry_integration_tests::original_purchase::request as invocation;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_economy::managed_entitlement::EntitlementProtocol;
use mush_market::mind_entitlement::{
    EntitlementPurchaseV1, EntitlementQuoteV1, EntitlementReceiptV1,
};
use mush_roll::commerce::{
    LicenceId,
    entitlement::{
        EntitlementAllocationV1, FiledRoyaltyTermsV1, PaidTermEconomicsV1, RoyaltyBasisV1,
    },
    purchase::PurchasePhase,
    records::{FiledRecord, InitialFilingOperation},
};
#[path = "owner_qualification_integration_tests.rs"]
mod owner_qualification;
#[path = "renewal_integration_tests.rs"]
mod renewal;

#[path = "derived_update_integration_tests.rs"]
mod derived_update;

const FIRST: u128 = 8_000_000;
type Events = Vec<(mush_core::Audience, mush_proto::OutputEvent)>;
fn action(world: &PlayerWorld, owner: &str, id: &str, version: u32) -> TestResult<DefinitionRef> {
    world
        .engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|entry| entry.reference())
        .find(|r| {
            r.key().owner.as_str() == owner
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == version
        })
        .cloned()
        .ok_or_else(|| format!("actual {owner}:{id}@{version}").into())
}
fn prepare(
    world: &mut PlayerWorld,
    owner: &str,
    id: &str,
    version: u32,
    actor: ObjId,
    number: u128,
    params: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    let action = action(world, owner, id, version)?;
    Ok(world
        .engine
        .prepare_registered_composed_authoritative_action(
            &action,
            &invocation(number, actor, owner, params)?,
            token(number),
        )?)
}
// Ordinary commerce steps exercise the same public unique-verb choice as +act.
// The separate direct prepare helper remains for owner-fence boundary tests.
#[allow(clippy::too_many_arguments)]
fn prepare_public(
    world: &mut PlayerWorld,
    owner: &str,
    id: &str,
    version: u32,
    actor: ObjId,
    number: u128,
    parameters: Value,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    let verb = match (owner, id, version) {
        ("market", "list_original_mind", 3) => "original-list-v3",
        ("market", "quote_original_mind", 3) => "original-quote-v3",
        ("market", "purchase_original_mind", 3) => "original-fund-v3",
        ("market", "settle_original_mind", 3) => "original-grant-v3",
        ("market", "refund_original_mind", 3) => "original-refund-v3",
        ("market", "list_derived_mind", 1) => "derived-list-v1",
        ("market", "quote_derived_mind", 1) => "derived-quote-v1",
        ("market", "purchase_derived_mind", 1) => "derived-fund-v1",
        ("market", "settle_derived_mind", 1) => "derived-grant-v1",
        ("market", "refund_derived_mind", 1) => "derived-refund-v1",
        ("market", "draft_mind_renewal", 1) => "mind-renewal-draft",
        ("market", "fund_mind_renewal", 1) => "mind-renewal-fund",
        ("market", "settle_mind_renewal", 1) => "mind-renewal-grant",
        ("market", "refund_mind_renewal", 1) => "mind-renewal-refund",
        ("market", "close_mind_renewal", 1) => "mind-renewal-close",
        _ => return prepare(world, owner, id, version, actor, number, parameters),
    };
    world.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x66; 32])?,
        [],
    )?);
    let exact = action(world, owner, id, version)?;
    let principal = PrincipalId::Object { id: actor };
    let query =
        world
            .engine
            .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = world.engine.project_legal_actions(&query)?;
    let projected = offers.projected();
    let mut matching = projected
        .iter()
        .filter(|offer| offer.presentation().verb.eq_ignore_ascii_case(verb));
    let offer = matching
        .next()
        .ok_or("signed personal licence public offer")?;
    assert!(
        matching.next().is_none(),
        "ambiguous actual personal licence line offer"
    );
    let req = invocation(number, actor, owner, parameters.clone())?;
    let resolved = world.engine.resolve_legal_action(
        &offers,
        mush_core::OfferExecutionContext {
            request: req.request(),
            principal,
            actor,
            idempotency: req.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        mush_core::OfferSubmission {
            offer: offer.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters,
        },
    )?;
    let (actual, request, _) = resolved.into_authoritative_parts();
    assert_eq!(actual, exact);
    Ok(world
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &exact,
            &request,
            token(number),
        )?)
}
fn commit(
    world: &mut PlayerWorld,
    owner: &str,
    id: &str,
    version: u32,
    actor: ObjId,
    number: u128,
    params: Value,
) -> TestResult {
    let (prepared, _) = prepare_public(world, owner, id, version, actor, number, params)?;
    let (writes, install) = prepared.into_parts();
    apply(world, &writes, install)
}
fn stage(
    world: &mut PlayerWorld,
    action: DefinitionRef,
    actor: ObjId,
    number: u128,
    params: Value,
    prepared: PreparedComposedTxn,
    output: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let inv = invocation(number, actor, "market", params)?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        output.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(number),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([91; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([92; 32]),
        action,
        provider: OwnerId::new("market")?,
        cause: mush_core::action::PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: prepared.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(number))),
        semantic_events,
        eligible_witnesses,
        durable_outbox,
    })?;
    let guards = world
        .engine
        .registry
        .server_guard_set_builder(mush_core::RevisionSet::default())?
        .finish()?;
    let _ = world.engine.world.take_dirty();
    world
        .engine
        .stage_authoritative_action(prepared, guards, bundle)?;
    Ok(())
}
fn allocation(world: &PlayerWorld, number: u128) -> TestResult<EntitlementAllocationV1> {
    Ok(serde_json::from_value(
        rows(&disk(world), "roll", "roll_mind_entitlement_purchase")
            .into_iter()
            .find(|row| row.get("operation") == Some(&json!(token(number))))
            .ok_or("actual entitlement allocation")?,
    )?)
}
fn own_term(world: &PlayerWorld, id: LicenceId) -> TestResult<PaidTermRecord> {
    Ok(serde_json::from_value(
        rows(&disk(world), "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|row| row.get("licence") == Some(&json!(id)))
            .ok_or("actual own paid term")?,
    )?)
}
fn balances(world: &PlayerWorld) -> TestResult<Vec<(WalletId, Amount, Amount)>> {
    Ok(world
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .snapshot()
        .accounts()
        .map(|a| (a.id, a.posted_balance, a.held_amount))
        .collect())
}
fn cash(world: &mut PlayerWorld) -> TestResult {
    for actor in [GOD, BUYER] {
        world.engine.world.move_to(actor, ROOM)?;
    }
    let economy = world
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?;
    let faucet_owner =
        EconomicOwner::System(SystemPrincipalId::new("purchase.fixture").ok_or("funding system")?);
    let faucet = economy
        .snapshot()
        .accounts()
        .find(|a| a.owner == faucet_owner)
        .ok_or("actual existing faucet")?
        .id;
    let before = economy
        .snapshot()
        .account(WalletId(0))
        .ok_or("buyer wallet")?
        .posted_balance;
    world.say(GOD, &format!("@econ/mint {faucet}=0 1000"))?;
    let after = world
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .snapshot()
        .account(WalletId(0))
        .ok_or("buyer cash")?
        .posted_balance;
    assert_eq!(
        after,
        before
            .checked_add(Amount::from_minor(1000))
            .ok_or("mint amount")?
    );
    assert!(after >= Amount::from_minor(100));
    Ok(())
}
fn file(
    f: &PurchaseFixture,
    certified: &Durable,
    rating: &RatingRef,
) -> TestResult<(Durable, LicenceId)> {
    let mut world = PlayerWorld::boot_entitlements(f, certified)?;
    let saved = source(&world)?;
    let parent: PaidTermRecord =
        serde_json::from_value(one(rows(&disk(&world), "roll", "roll_mind_paid_term"))?)?;
    assert!(matches!(parent.terms().form(), PaidForm::OpenResale {}));
    let measured: RatingReceipt = serde_json::from_value(
        rows(&disk(&world), "roll", "rating_receipts")
            .into_iter()
            .find(|r| r.pointer("/publication/job") == Some(&json!(rating.job())))
            .ok_or("actual complete marked receipt")?,
    )?;
    assert_eq!(measured.reference()?, *rating);
    assert_eq!(measured.subject().revision().revision(), saved.reference());
    commit(
        &mut world,
        "roll",
        mush_core::reserved_shell::RESERVE_ACTION,
        1,
        BUYER,
        FIRST,
        json!({}),
    )?;
    let rev = saved.reference();
    let params = json!({"shell_request":RequestUid::from_bytes(FIRST.to_be_bytes()),"mind":rev.key.mind,"revision":rev.key.number,"revision_digest":rev.digest,
        "rating_job":rating.job(),"rating_id":rating.id(),"rating_digest":rating.receipt_digest(),"payer":WalletId(2),"charter_name":"Actual paid derivative"});
    assert!(
        prepare(
            &mut world,
            "roll",
            "initial_filing",
            1,
            BUYER,
            FIRST + 1,
            params.clone()
        )
        .is_err()
    );
    commit(
        &mut world,
        "roll",
        "dependent_initial_filing",
        1,
        BUYER,
        FIRST + 1,
        params,
    )?;
    let op: InitialFilingOperation = serde_json::from_value(
        rows(&disk(&world), "roll", "roll_mind_initial_filing")
            .into_iter()
            .find(|r| r.get("commit") == Some(&json!(token(FIRST + 1))))
            .ok_or("actual filed operation")?,
    )?;
    let value = serde_json::to_value(op)?;
    let owner =
        serde_json::from_value::<LicenceId>(value.get("licence").ok_or("owner licence")?.clone())?;
    let snapshot = capture(&world, &disk(&world));
    let _cold = PlayerWorld::boot_entitlements(f, &snapshot)?;
    Ok((snapshot, owner))
}
fn listing_params(owner: LicenceId, form: &str, rate: u16) -> Value {
    json!({"owner_licence":owner.get().to_string(),"form":form,"royalty_permille":rate})
}
fn list(world: &mut PlayerWorld, owner: LicenceId, number: u128) -> TestResult {
    commit(
        world,
        "market",
        "list_derived_mind",
        1,
        BUYER,
        number,
        listing_params(owner, "open_resale", 100),
    )
}
fn quote(world: &mut PlayerWorld, listing: u128, number: u128) -> TestResult {
    commit(
        world,
        "market",
        "quote_derived_mind",
        1,
        GOD,
        number,
        json!({"listing":token(listing)}),
    )
}
fn fund(world: &mut PlayerWorld, quote: u128, number: u128) -> TestResult {
    commit(
        world,
        "market",
        "purchase_derived_mind",
        1,
        GOD,
        number,
        json!({"quote":token(quote)}),
    )
}
fn grant(world: &mut PlayerWorld, operation: u128, number: u128) -> TestResult {
    commit(
        world,
        "market",
        "settle_derived_mind",
        1,
        GOD,
        number,
        json!({"operation":token(operation)}),
    )
}
fn title_fences(f: &PurchaseFixture, filed: &Durable, owner: LicenceId) -> TestResult {
    for after_stage in [false, true] {
        let mut world = PlayerWorld::boot_entitlements(f, filed)?;
        cash(&mut world)?;
        let child: FiledRecord = serde_json::from_value(
            rows(&disk(&world), "roll", "roll_mind_filed")
                .into_iter()
                .find(|r| r.get("operation") == Some(&json!(token(FIRST + 1))))
                .ok_or("child")?,
        )?;
        let charter = child.content().position().charter.object();
        let params = listing_params(owner, "open_resale", 100);
        let number = FIRST + 50;
        let definition = action(&world, "market", "list_derived_mind", 1)?;
        let (prepared, output) = prepare(
            &mut world,
            "market",
            "list_derived_mind",
            1,
            BUYER,
            number,
            params.clone(),
        )?;
        // Core admits one pending commit: no title transfer can land between
        // a prepared (or staged) listing and its install, so the transfer is
        // refused, the seller keeps the title and no listing row is written.
        if after_stage {
            stage(
                &mut world, definition, BUYER, number, params, prepared, output,
            )?;
            assert!(
                world.transfer_charter(charter, GOD, 49_000_080).is_err(),
                "title moved under a staged listing"
            );
        } else {
            assert!(
                world.transfer_charter(charter, GOD, 49_000_090).is_err(),
                "title moved under a prepared listing"
            );
            drop((definition, params, prepared, output));
        }
        assert_eq!(
            world
                .engine
                .world
                .module_state::<EconomyStore>()
                .ok_or("economy")?
                .snapshot()
                .asset(charter)
                .ok_or("title")?
                .title_owner,
            EconomicOwner::Object(BUYER)
        );
        assert!(rows(&disk(&world), "market", "market_mind_entitlement_listing").is_empty());
        assert!(rows(&disk(&world), "roll", "roll_mind_filed_royalty_terms").is_empty());
    }
    Ok(())
}
fn merge_login_accounts(f: &PurchaseFixture) -> TestResult<mush_core::AccountId> {
    let mut values = f.accounts.values.lock().map_err(module_error)?;
    let requester = *values.get(&GOD).ok_or("requester login account")?;
    let seller = *values.get(&BUYER).ok_or("seller login account")?;
    assert_ne!(requester, seller);
    f.accounts.epoch.invalidate();
    values.insert(BUYER, requester);
    Ok(seller)
}
fn restore_login_account(f: &PurchaseFixture, seller: mush_core::AccountId) -> TestResult {
    let mut values = f.accounts.values.lock().map_err(module_error)?;
    f.accounts.epoch.invalidate();
    values.insert(BUYER, seller);
    Ok(())
}
fn account_fences(
    f: &PurchaseFixture,
    saved: &Durable,
    id: &str,
    number: u128,
    params: Value,
) -> TestResult {
    for after_prepare in [false, true] {
        let mut world = PlayerWorld::boot_entitlements(f, saved)?;
        // A fresh boot seats both actors in the lobby; the venue is ROOM, so
        // each fence starts where the purchase itself is made.
        for actor in [GOD, BUYER] {
            world.engine.world.move_to(actor, ROOM)?;
        }
        world.commit()?;
        let before = balances(&world)?;
        let original_disk = disk(&world);
        let refused = if after_prepare {
            let definition = action(&world, "market", id, 1)?;
            let (prepared, output) =
                prepare(&mut world, "market", id, 1, GOD, number, params.clone())?;
            let seller = merge_login_accounts(f)?;
            let refused = stage(
                &mut world,
                definition,
                GOD,
                number,
                params.clone(),
                prepared,
                output,
            )
            .is_err();
            restore_login_account(f, seller)?;
            refused
        } else {
            let seller = merge_login_accounts(f)?;
            let refused =
                prepare(&mut world, "market", id, 1, GOD, number, params.clone()).is_err();
            restore_login_account(f, seller)?;
            refused
        };
        assert!(
            refused,
            "current or retained account capture must refuse {id}"
        );
        assert_eq!(balances(&world)?, before);
        assert!(
            disk(&world) == original_disk,
            "account refusal changed retained rows"
        );
    }
    Ok(())
}
fn transactions(
    f: &PurchaseFixture,
    filed: &Durable,
    owner: LicenceId,
) -> TestResult<(Durable, EntitlementAllocationV1)> {
    let mut world = PlayerWorld::boot_entitlements(f, filed)?;
    cash(&mut world)?;
    let now = world.engine.world.now().checked_add(17).ok_or("time")?;
    world.engine.world.set_now(now);
    let before = balances(&world)?;
    list(&mut world, owner, FIRST + 2)?;
    quote(&mut world, FIRST + 2, FIRST + 3)?;
    let q: EntitlementQuoteV1 = serde_json::from_value(one(rows(
        &disk(&world),
        "market",
        "market_mind_entitlement_quote",
    ))?)?;
    let qv = serde_json::to_value(&q)?;
    assert_eq!(qv.pointer("/product/protocol"), Some(&json!("derived_v1")));
    assert_eq!(
        qv.pointer("/product/grant/downstream/royalty_permille"),
        Some(&json!(100))
    );
    let chain: FiledRoyaltyTermsV1 =
        serde_json::from_value(qv.get("royalty").ok_or("royalty")?.clone())?;
    assert_eq!(chain.edges().len(), 1);
    let edge = chain.edges().first().ok_or("actual acquisition")?;
    assert_eq!(edge.royalty_permille(), 0);
    assert!(matches!(
        edge.basis(),
        RoyaltyBasisV1::LegacySourceFormsV2Zero { .. }
    ));
    let quoted = capture(&world, &disk(&world));
    account_fences(
        f,
        &quoted,
        "purchase_derived_mind",
        FIRST + 80,
        json!({"quote":token(FIRST+3)}),
    )?;
    // E4 and typed images remain unchanged before the actual owner install.
    eprintln!("M7_STORY_DIAG {} god_room={:?} room={:?}", "main purchase", world.engine.world.get(GOD).and_then(|o| o.location), ROOM); // TEMP M7 diag
    let (prepared, _) = prepare(
        &mut world,
        "market",
        "purchase_derived_mind",
        1,
        GOD,
        FIRST + 4,
        json!({"quote":token(FIRST+3)}),
    )?;
    assert_eq!(balances(&world)?, before);
    assert!(rows(&disk(&world), "roll", "roll_mind_entitlement_purchase").is_empty());
    let (writes, install) = prepared.into_parts();
    apply(&mut world, &writes, install)?;
    let funded = capture(&world, &disk(&world));
    account_fences(
        f,
        &funded,
        "settle_derived_mind",
        FIRST + 81,
        json!({"operation":token(FIRST+4)}),
    )?;
    let mut world = PlayerWorld::boot_entitlements(f, &funded)?;
    for actor in [GOD, BUYER] {
        world.engine.world.move_to(actor, ROOM)?;
    }
    assert_eq!(
        allocation(&world, FIRST + 4)?.phase(),
        PurchasePhase::Funded {}
    );
    grant(&mut world, FIRST + 4, FIRST + 5)?;
    let after = balances(&world)?;
    for (id, posted, held) in &before {
        let delta = match id.0 {
            0 => -100,
            1 => 10,
            2 => 90,
            _ => 0,
        };
        let actual = after
            .iter()
            .find(|(key, _, _)| key == id)
            .ok_or("retained account")?;
        assert_eq!(
            actual.1,
            posted
                .checked_add(Amount::from_minor(delta))
                .ok_or("exact payout")?
        );
        assert_eq!(actual.2, *held);
    }
    let a = allocation(&world, FIRST + 4)?;
    assert_eq!(a.protocol(), EntitlementProtocol::DerivedV1);
    assert_eq!(
        a.phase(),
        PurchasePhase::Granted {
            commit: token(FIRST + 5)
        }
    );
    assert_eq!(a.holder(), GOD);
    let term = own_term(&world, a.licence())?;
    let parent: PaidTermRecord = serde_json::from_value(
        rows(&disk(&world), "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|r| r.get("operation") != Some(&json!(a.operation())))
            .ok_or("parent term")?,
    )?;
    assert!(term.window().grace_end().get() > parent.window().grace_end().get());
    let economics: PaidTermEconomicsV1 = serde_json::from_value(one(rows(
        &disk(&world),
        "roll",
        "roll_mind_paid_term_economics",
    ))?)?;
    assert_eq!(&economics, a.economics());
    assert_eq!(economics.grant().renewal().price(), Amount::from_minor(73));
    let receipt: EntitlementReceiptV1 = serde_json::from_value(one(rows(
        &disk(&world),
        "market",
        "market_mind_entitlement_receipt",
    ))?)?;
    assert_eq!(receipt.operation(), a.operation());
    let purchase: EntitlementPurchaseV1 = serde_json::from_value(one(rows(
        &disk(&world),
        "market",
        "market_mind_entitlement_purchase",
    ))?)?;
    assert_eq!(purchase.phase(), a.phase());
    let actual = world
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .purchase_receipt_audit()
        .get(a.operation())?;
    assert!(actual.terminal().ok_or("actual E4 terminal")?.granted());
    let snapshot = capture(&world, &disk(&world));
    let cold = PlayerWorld::boot_entitlements(f, &snapshot)?;
    assert_eq!(balances(&cold)?, balances(&world)?);
    assert!(
        PlayerWorld::boot_dependent(f, &snapshot).is_err(),
        "old-only composition cannot accept new paid history"
    );
    // Each half of the immutable pair is mandatory, including all-row loss.
    for (owner_ns, family) in [
        ("market", "market_mind_entitlement_purchase"),
        ("market", "market_mind_entitlement_receipt"),
        ("market", "market_mind_entitlement_listing"),
        ("roll", "roll_mind_paid_term_economics"),
        ("roll", "roll_mind_filed_royalty_terms"),
    ] {
        let mut missing = snapshot.clone();
        missing.disk.retain(|(ns, _), row| {
            !(ns == owner_ns && row.pointer("/family/id") == Some(&json!(family)))
        });
        assert!(
            PlayerWorld::boot_entitlements(f, &missing).is_err(),
            "missing {family}"
        );
    }
    Ok((snapshot, a))
}
fn original_v3(f: &PurchaseFixture, filed: &Durable) -> TestResult {
    let mut world = PlayerWorld::boot_entitlements(f, filed)?;
    cash(&mut world)?;
    commit(
        &mut world,
        "market",
        "list_original_mind",
        3,
        GOD,
        FIRST + 60,
        listing_params(LicenceId::new(1)?, "open_resale", 150),
    )?;
    commit(
        &mut world,
        "market",
        "quote_original_mind",
        3,
        BUYER,
        FIRST + 61,
        json!({"listing":token(FIRST+60)}),
    )?;
    commit(
        &mut world,
        "market",
        "purchase_original_mind",
        3,
        BUYER,
        FIRST + 62,
        json!({"quote":token(FIRST+61)}),
    )?;
    commit(
        &mut world,
        "market",
        "settle_original_mind",
        3,
        BUYER,
        FIRST + 63,
        json!({"operation":token(FIRST+62)}),
    )?;
    let a = allocation(&world, FIRST + 62)?;
    assert_eq!(a.protocol(), EntitlementProtocol::OriginalV3);
    assert_eq!(
        a.economics().grant().downstream().royalty_permille(),
        Some(150)
    );
    assert_eq!(
        a.economics().grant().renewal().price(),
        Amount::from_minor(73)
    );
    assert_eq!(a.terms().form(), PaidForm::OpenResale {});
    let snapshot = capture(&world, &disk(&world));
    let _ = PlayerWorld::boot_entitlements(f, &snapshot)?;
    renewal::service_tests::unavailable_buyer(f, &snapshot, &a)?;
    resale_routes::exercise(
        &f.accounts,
        &snapshot,
        a.licence(),
        BUYER,
        false,
        "purchase.fixture",
        63_000_000,
        |saved| PlayerWorld::boot_paid_resales(f, saved),
    )?;
    Ok(())
}
fn lapse_refund(f: &PurchaseFixture, filed: &Durable, owner: LicenceId) -> TestResult {
    let mut world = PlayerWorld::boot_entitlements(f, filed)?;
    cash(&mut world)?;
    let parent: PaidTermRecord =
        serde_json::from_value(one(rows(&disk(&world), "roll", "roll_mind_paid_term"))?)?;
    world.engine.world.set_now(
        parent
            .window()
            .grace_end()
            .get()
            .checked_sub(1)
            .ok_or("before expiry")?,
    );
    list(&mut world, owner, FIRST + 70)?;
    quote(&mut world, FIRST + 70, FIRST + 71)?;
    let before = balances(&world)?;
    fund(&mut world, FIRST + 71, FIRST + 72)?;
    world
        .engine
        .world
        .set_now(parent.window().grace_end().get());
    assert!(list(&mut world, owner, FIRST + 73).is_err());
    assert!(grant(&mut world, FIRST + 72, FIRST + 74).is_err()); // Quote is only one second old.
    commit(
        &mut world,
        "market",
        "refund_derived_mind",
        1,
        GOD,
        FIRST + 75,
        json!({"operation":token(FIRST+72)}),
    )?;
    assert_eq!(balances(&world)?, before);
    let a = allocation(&world, FIRST + 72)?;
    assert_eq!(
        a.phase(),
        PurchasePhase::Refunded {
            commit: token(FIRST + 75)
        }
    );
    assert!(
        !rows(&disk(&world), "roll", "roll_mind_personal_paid_licence")
            .iter()
            .any(|r| r.get("id") == Some(&json!(a.licence())))
    );
    assert!(rows(&disk(&world), "roll", "roll_mind_paid_term_economics").is_empty());
    let snapshot = capture(&world, &disk(&world));
    let _ = PlayerWorld::boot_entitlements(f, &snapshot)?;
    Ok(())
}
fn source_answer(
    world: &mut PlayerWorld,
    actor: ObjId,
    licence: LicenceId,
    expected: Option<mush_core::DefinitionDigest>,
) -> TestResult {
    use mush_proto::{InvokeRequest, InvokeResult, MindRequest, MindResponse, OutputEvent};
    let conn = if actor == BUYER {
        BUYER_CONN
    } else {
        OBSERVER_CONN
    };
    let mut out = Outbox::new();
    world.engine.step(
        Inbound::Invoke {
            conn,
            actor,
            id: 8_000,
            request: InvokeRequest::Mind(MindRequest::FiledSource {
                licence: licence.get().to_string(),
                first: 1,
                last: 200,
            }),
        },
        &mut out,
        &mut mush_core::engine::Effects::new(),
    );
    let events: Events = out.take();
    world.commit()?;
    let mut answers = 0;
    for (audience, event) in &events {
        if let OutputEvent::InvokeResult {
            result: InvokeResult::Mind(response),
            ..
        } = event
        {
            answers += 1;
            assert_eq!(*audience, mush_core::Audience::Conn(conn));
            match response {
                MindResponse::FiledSource {
                    source_sha256,
                    lines,
                    ..
                } => {
                    assert_eq!(Some(source_sha256.clone()), expected.map(|h| h.to_hex()));
                    assert!(!lines.is_empty());
                }
                MindResponse::Error { .. } => assert!(expected.is_none()),
                _ => return Err("unexpected paid source response".into()),
            }
        }
        if serde_json::to_string(event)?.contains("this.landed") {
            assert!(expected.is_some());
            assert_eq!(*audience, mush_core::Audience::Conn(conn));
        }
    }
    assert_eq!(answers, 1);
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
            .find(|r| r.get("operation") != Some(&json!(a.operation())))
            .ok_or("parent")?,
    )?;
    let mut lapsed = granted.clone();
    lapsed.now = parent.window().grace_end().get();
    let mut world = PlayerWorld::boot_entitlements(f, &lapsed)?;
    let tasks = world.tasks.clone();
    let result:TestResult=async {
        let own=own_term(&world,a.licence())?;assert!(world.engine.world.now()<own.window().grace_end().get());
        source_answer(&mut world,GOD,a.licence(),Some(a.revision().source()))?;
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
        let mut fresh=PlayerWorld::boot_entitlements(f,&ended)?;source_answer(&mut fresh,GOD,a.licence(),None)?;
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
    certified: &Durable,
    rating: &RatingRef,
) -> TestResult {
    eprintln!("M7_STORY phase=entitlement.file event=begin");
    let (filed, owner) = file(f, certified, rating)?;
    eprintln!("M7_STORY_DIAG entitlement step=filed"); // TEMP M7 diag
    title_fences(f, &filed, owner)?;
    eprintln!("M7_STORY_DIAG entitlement step=title_fences"); // TEMP M7 diag
    original_v3(f, &filed)?;
    eprintln!("M7_STORY_DIAG entitlement step=original_v3"); // TEMP M7 diag
    lapse_refund(f, &filed, owner)?;
    eprintln!("M7_STORY_DIAG entitlement step=lapse_refund"); // TEMP M7 diag
    let (granted, allocation) = transactions(f, &filed, owner)?;
    resale_routes::exercise(
        &f.accounts,
        &granted,
        allocation.licence(),
        GOD,
        false,
        "purchase.fixture",
        64_000_000,
        |saved| PlayerWorld::boot_paid_resales(f, saved),
    )?;
    story_phase("derived.compatible_update", || {
        derived_update::exercise(f, &granted, owner, &allocation)
    })?
    .await?;
    story_phase("owner.qualification", || {
        owner_qualification::exercise(f, &granted, owner, &allocation)
    })?
    .await?;
    story_phase("author.withdrawal", || {
        withdrawal::exercise(f, &granted, owner, &allocation)
    })?
    .await?;
    story_phase("registry.recall", || {
        registry_recall::exercise(f, &granted, owner, &allocation)
    })?
    .await?;
    story_phase("registry.revocation", || {
        registry_revocation::exercise(f, &granted, owner, &allocation)
    })?
    .await?;
    story_phase("minds.facts", || {
        minds_facts::exercise(f, &granted, owner, &allocation)
    })?
    .await?;
    story_phase("renewal", || {
        renewal::exercise(f, &granted, owner, &allocation)
    })?
    .await?;
    story_phase("entitlement.customer", || {
        customer(f, &granted, owner, &allocation)
    })?
    .await?;
    eprintln!("M7_STORY phase=entitlement.file event=complete");
    Ok(())
}
