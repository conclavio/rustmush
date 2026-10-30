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
// Doc 17h §3 M8f: the procurement tick end to end.
#[path = "procurement_integration_tests.rs"]
mod procurement;

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
    let offer = matching.next().ok_or_else(|| {
        let verbs: Vec<_> = projected.iter().map(|o| o.presentation().verb.to_string()).collect();
        let room = world.engine.world.get(actor).and_then(|o| o.location);
        format!("signed personal licence public offer: wanted {verb} for {id} as {actor:?} in {room:?}; projected {verbs:?}")
    })?;
    check!(
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
    check_eq!(actual, exact);
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
    check_eq!(
        after,
        before
            .checked_add(Amount::from_minor(1000))
            .ok_or("mint amount")?
    );
    check!(after >= Amount::from_minor(100));
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
    check!(matches!(parent.terms().form(), PaidForm::OpenResale {}));
    let measured: RatingReceipt = serde_json::from_value(
        rows(&disk(&world), "roll", "rating_receipts")
            .into_iter()
            .find(|r| r.pointer("/publication/job") == Some(&json!(rating.job())))
            .ok_or("actual complete marked receipt")?,
    )?;
    check_eq!(measured.reference()?, *rating);
    check_eq!(measured.subject().revision().revision(), saved.reference());
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
    check!(
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
/// A fresh boot seats both actors in the lobby; the market venue is ROOM.
fn seat(world: &mut PlayerWorld) -> TestResult {
    for actor in [GOD, BUYER] {
        world.engine.world.move_to(actor, ROOM)?;
    }
    world.commit()
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
            check!(
                world.transfer_charter(charter, GOD, 49_000_080).is_err(),
                "title moved under a staged listing"
            );
        } else {
            check!(
                world.transfer_charter(charter, GOD, 49_000_090).is_err(),
                "title moved under a prepared listing"
            );
            drop((definition, params, prepared, output));
        }
        check_eq!(
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
        check!(rows(&disk(&world), "market", "market_mind_entitlement_listing").is_empty());
        check!(rows(&disk(&world), "roll", "roll_mind_filed_royalty_terms").is_empty());
    }
    Ok(())
}
fn merge_login_accounts(f: &PurchaseFixture) -> TestResult<mush_core::AccountId> {
    let mut values = f.accounts.values.lock().map_err(module_error)?;
    let requester = *values.get(&GOD).ok_or("requester login account")?;
    let seller = *values.get(&BUYER).ok_or("seller login account")?;
    check_ne!(requester, seller);
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
        check!(
            refused,
            "current or retained account capture must refuse {id}"
        );
        check_eq!(balances(&world)?, before);
        check!(
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
    check_eq!(qv.pointer("/product/protocol"), Some(&json!("derived_v1")));
    check_eq!(
        qv.pointer("/product/grant/downstream/royalty_permille"),
        Some(&json!(100))
    );
    let chain: FiledRoyaltyTermsV1 =
        serde_json::from_value(qv.get("royalty").ok_or("royalty")?.clone())?;
    check_eq!(chain.edges().len(), 1);
    let edge = chain.edges().first().ok_or("actual acquisition")?;
    check_eq!(edge.royalty_permille(), 0);
    check!(matches!(
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
    let (prepared, _) = prepare(
        &mut world,
        "market",
        "purchase_derived_mind",
        1,
        GOD,
        FIRST + 4,
        json!({"quote":token(FIRST+3)}),
    )?;
    check_eq!(balances(&world)?, before);
    check!(rows(&disk(&world), "roll", "roll_mind_entitlement_purchase").is_empty());
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
    check_eq!(
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
        check_eq!(
            actual.1,
            posted
                .checked_add(Amount::from_minor(delta))
                .ok_or("exact payout")?
        );
        check_eq!(actual.2, *held);
    }
    let a = allocation(&world, FIRST + 4)?;
    check_eq!(a.protocol(), EntitlementProtocol::DerivedV1);
    check_eq!(
        a.phase(),
        PurchasePhase::Granted {
            commit: token(FIRST + 5)
        }
    );
    check_eq!(a.holder(), mush_economy::Holder::Character(GOD));
    let term = own_term(&world, a.licence())?;
    let parent: PaidTermRecord = serde_json::from_value(
        rows(&disk(&world), "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|r| r.get("operation") != Some(&json!(a.operation())))
            .ok_or("parent term")?,
    )?;
    check!(term.window().grace_end().get() > parent.window().grace_end().get());
    let economics: PaidTermEconomicsV1 = serde_json::from_value(one(rows(
        &disk(&world),
        "roll",
        "roll_mind_paid_term_economics",
    ))?)?;
    check_eq!(&economics, a.economics());
    check_eq!(economics.grant().renewal().price(), Amount::from_minor(73));
    let receipt: EntitlementReceiptV1 = serde_json::from_value(one(rows(
        &disk(&world),
        "market",
        "market_mind_entitlement_receipt",
    ))?)?;
    check_eq!(receipt.operation(), a.operation());
    let purchase: EntitlementPurchaseV1 = serde_json::from_value(one(rows(
        &disk(&world),
        "market",
        "market_mind_entitlement_purchase",
    ))?)?;
    check_eq!(purchase.phase(), a.phase());
    let actual = world
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .purchase_receipt_audit()
        .get(a.operation())?;
    check!(actual.terminal().ok_or("actual E4 terminal")?.granted());
    let snapshot = capture(&world, &disk(&world));
    let cold = PlayerWorld::boot_entitlements(f, &snapshot)?;
    check_eq!(balances(&cold)?, balances(&world)?);
    check!(
        crate::test_checks::refused(PlayerWorld::boot_dependent(f, &snapshot))?,
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
        check!(
            crate::test_checks::refused(PlayerWorld::boot_entitlements(f, &missing))?,
            "missing {family}"
        );
    }
    Ok((snapshot, a))
}
/// M8c B2/B3 (the brief's test 3, the composition half): GOD, granted
/// `roll.holder.org` on the Guild's root scope, quotes BUYER's derived
/// listing for the Guild, pays from the Guild's treasury (its one account on
/// its root scope, the hold the organisation's) and completes it; the
/// allocation's holder is `Organization { org, acting: GOD }` and the split
/// is the usual one (the Registry's 10, the seller's 90) off the treasury,
/// GOD's own wallet untouched. Before the grant, for an unknown
/// organisation, and for a character of the seller's login acting for the
/// Guild (D-UM-40 through the lobby oracle), the quote is refused.
fn store(world: &PlayerWorld) -> TestResult<&EconomyStore> {
    Ok(world
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?)
}
fn org_buys(f: &PurchaseFixture, granted: &Durable, owner: LicenceId) -> TestResult {
    const ORG: u128 = FIRST + 900;
    let mut world = PlayerWorld::boot_entitlements(f, granted)?;
    cash(&mut world)?;
    world.say(GOD, "@org/create Guild")?;
    let (org, root) = {
        let store = world
            .engine
            .world
            .module_state::<mush_org::OrgStore>()
            .ok_or("Org")?
            .state();
        let guild = store
            .orgs()
            .find(|o| o.name.as_str() == "Guild")
            .ok_or("the Guild")?;
        (guild.id, guild.root_scope)
    };
    let instrument = store(&world)?
        .snapshot()
        .account(WalletId(0))
        .ok_or("buyer wallet")?
        .instrument
        .clone();
    world.say(
        GOD,
        &format!("@account/open org:Guild/{}={}", root.0, instrument.as_str()),
    )?;
    let treasury = store(&world)?
        .snapshot()
        .accounts()
        .find(|a| a.owner == EconomicOwner::Organization(org) && a.scope == Some(root))
        .ok_or("the Guild's treasury")?
        .id;
    let fixture =
        EconomicOwner::System(SystemPrincipalId::new("purchase.fixture").ok_or("funding system")?);
    let faucet = store(&world)?
        .snapshot()
        .accounts()
        .find(|a| a.owner == fixture)
        .ok_or("actual existing faucet")?
        .id;
    world.say(GOD, &format!("@econ/mint {faucet}={treasury} 1000"))?;
    let balance = |world: &PlayerWorld, id: WalletId| -> TestResult<(Amount, Amount)> {
        let a = world
            .engine
            .world
            .module_state::<EconomyStore>()
            .ok_or("economy")?
            .snapshot()
            .account(id)
            .ok_or("account")?;
        Ok((a.posted_balance, a.held_amount))
    };
    check_eq!(
        balance(&world, treasury)?,
        (Amount::from_minor(1000), Amount::ZERO)
    );
    list(&mut world, owner, ORG + 1)?;
    let quote_params = json!({"listing": token(ORG + 1), "org": org.0});
    check!(
        crate::test_checks::refused(prepare_public(
            &mut world,
            "market",
            "quote_derived_mind",
            1,
            GOD,
            ORG + 2,
            quote_params.clone()
        ))?,
        "ungranted: refused"
    );
    check!(
        crate::test_checks::refused(prepare_public(
            &mut world,
            "market",
            "quote_derived_mind",
            1,
            GOD,
            ORG + 3,
            json!({"listing": token(ORG + 1), "org": org.0 + 1000})
        ))?,
        "an unknown organisation: refused"
    );
    world.say(
        GOD,
        &format!(
            "@org/grant #{}=Guild {}",
            GOD.0,
            mush_roll::commerce::org_holder::HOLDER_ORG_CAPABILITY
        ),
    )?;
    let seller = merge_login_accounts(f)?;
    let alt_refused = crate::test_checks::refused(prepare_public(
        &mut world,
        "market",
        "quote_derived_mind",
        1,
        GOD,
        ORG + 4,
        quote_params.clone(),
    ))?;
    restore_login_account(f, seller)?;
    check!(
        alt_refused,
        "the seller's login acting for the Guild: refused"
    );
    let before = balances(&world)?;
    commit(
        &mut world,
        "market",
        "quote_derived_mind",
        1,
        GOD,
        ORG + 5,
        quote_params,
    )?;
    let quote = rows(&disk(&world), "market", "market_mind_entitlement_quote")
        .into_iter()
        .find(|v| v.get("publication") == Some(&json!(token(ORG + 5))))
        .ok_or("the Guild's quote")?;
    check_eq!(
        quote.get("buyer"),
        Some(&json!({"org": org, "acting": GOD})),
        "the holder is the organisation, GOD acting"
    );
    check_eq!(quote.get("payer"), Some(&json!(treasury)));
    commit(
        &mut world,
        "market",
        "purchase_derived_mind",
        1,
        GOD,
        ORG + 6,
        json!({"quote": token(ORG + 5)}),
    )?;
    check_eq!(
        balance(&world, treasury)?,
        (Amount::from_minor(1000), Amount::from_minor(100)),
        "the hold is the treasury's"
    );
    check_eq!(
        allocation(&world, ORG + 6)?.phase(),
        PurchasePhase::Funded {}
    );
    commit(
        &mut world,
        "market",
        "settle_derived_mind",
        1,
        GOD,
        ORG + 7,
        json!({"operation": token(ORG + 6)}),
    )?;
    check_eq!(
        balance(&world, treasury)?,
        (Amount::from_minor(900), Amount::ZERO)
    );
    let after = balances(&world)?;
    for (id, posted, held) in &before {
        let delta = match id.0 {
            1 => 10,
            2 => 90,
            _ if *id == treasury => -100,
            _ => 0,
        };
        let actual = after
            .iter()
            .find(|(key, _, _)| key == id)
            .ok_or("retained account")?;
        check_eq!(
            actual.1,
            posted
                .checked_add(Amount::from_minor(delta))
                .ok_or("exact payout")?,
            "account {}",
            id.0
        );
        check_eq!(actual.2, *held);
    }
    let a = allocation(&world, ORG + 6)?;
    check_eq!(
        a.holder(),
        mush_economy::Holder::Organization { org, acting: GOD }
    );
    check_eq!(
        a.phase(),
        PurchasePhase::Granted {
            commit: token(ORG + 7)
        }
    );
    check!(
        disk(&world)
            .keys()
            .any(|(ns, key)| ns == "roll" && key.contains(&format!("org/{:020}/licence/", org.0))),
        "the organisation's holding is indexed under the organisation"
    );
    let snapshot = capture(&world, &disk(&world));
    let cold = PlayerWorld::boot_entitlements(f, &snapshot)?;
    check_eq!(balances(&cold)?, balances(&world)?);
    check_eq!(
        allocation(&cold, ORG + 6)?.holder(),
        mush_economy::Holder::Organization { org, acting: GOD }
    );
    Ok(())
}
/// M8c F2/F4/F8/F11: organisation 0 buys through a non-staff member, and a
/// second member renews what it holds.
/// - The member (a real player with its own login account and wallet) is
///   granted `roll.holder.org`; a grant limited below the price refuses the
///   Quote, one at the price quotes;
/// - the grant revoked between the Quote and the Fund refuses the Fund
///   (the grant is re-asked at every act); re-granted, the Fund places the
///   treasury's hold as the organisation itself;
/// - M8c S1: the quoting member's grant revoked after the Fund, a second
///   non-staff member refunds the organisation's escrow (the economy pins
///   the act to a live member whose principal matches, the market re-asks
///   the grant, and the second member finds the organisation's row);
/// - doc 17h §3 M8f P2a: the Grant asks with the price, so a grant limited
///   below it refuses the Grant and one limited at the price completes a
///   second purchase, the organisation holding the licence with the member
///   acting;
/// - S3: the second member, a non-staff character who never bought,
///   renews it end to end (Draft, Fund, Grant): the treasury pays, the
///   renewed pointer names the second member as acting, the holdings row
///   keeps the first and both boot audits accept them by principal on a
///   cold boot.
fn org_members(f: &PurchaseFixture, granted: &Durable, owner: LicenceId) -> TestResult {
    const ORG: u128 = FIRST + 950;
    let mut world = PlayerWorld::boot_entitlements(f, granted)?;
    cash(&mut world)?;
    world.say(GOD, "@org/create Guild")?;
    let (org, root) = {
        let store = world
            .engine
            .world
            .module_state::<mush_org::OrgStore>()
            .ok_or("Org")?
            .state();
        let guild = store
            .orgs()
            .find(|o| o.name.as_str() == "Guild")
            .ok_or("the Guild")?;
        (guild.id, guild.root_scope)
    };
    check_eq!(org, mush_org::OrgId(0), "F8: the Guild is organisation 0");
    let instrument = store(&world)?
        .snapshot()
        .account(WalletId(0))
        .ok_or("buyer wallet")?
        .instrument
        .clone();
    world.say(
        GOD,
        &format!("@account/open org:Guild/{}={}", root.0, instrument.as_str()),
    )?;
    let treasury = store(&world)?
        .snapshot()
        .accounts()
        .find(|a| a.owner == EconomicOwner::Organization(org) && a.scope == Some(root))
        .ok_or("the Guild's treasury")?
        .id;
    let fixture =
        EconomicOwner::System(SystemPrincipalId::new("purchase.fixture").ok_or("funding system")?);
    let faucet = store(&world)?
        .snapshot()
        .accounts()
        .find(|a| a.owner == fixture)
        .ok_or("actual existing faucet")?
        .id;
    world.say(GOD, &format!("@econ/mint {faucet}={treasury} 1000"))?;
    // A non-staff member: a real player with a login account of its own and
    // a personal wallet.
    let mut members = Vec::new();
    for (name, login) in [
        ("Guild Member", "guild-member"),
        ("Guild Second", "guild-second"),
    ] {
        let id = world
            .engine
            .world
            .create(mush_core::ObjKind::Player, name, GOD, Some(ROOM))?;
        let mut values = f.accounts.values.lock().map_err(module_error)?;
        f.accounts.epoch.invalidate();
        values.insert(id, mush_core::AccountId::mint(login, 0)?);
        members.push(id);
    }
    let pair = (members[0], members[1]);
    let result = org_members_with(f, &mut world, pair, (org, treasury), owner, ORG);
    {
        let mut values = f.accounts.values.lock().map_err(module_error)?;
        f.accounts.epoch.invalidate();
        for id in members {
            values.remove(&id);
        }
    }
    result
}
fn org_members_with(
    f: &PurchaseFixture,
    world: &mut PlayerWorld,
    (member, second): (ObjId, ObjId),
    (org, treasury): (mush_org::OrgId, WalletId),
    owner: LicenceId,
    base: u128,
) -> TestResult {
    let instrument = store(world)?
        .snapshot()
        .account(treasury)
        .ok_or("treasury")?
        .instrument
        .clone();
    for who in [member, second] {
        world.say(
            GOD,
            &format!("@account/open #{}={}", who.0, instrument.as_str()),
        )?;
    }
    list(world, owner, base + 1)?;
    let quote_params = json!({"listing": token(base + 1), "org": org.0});
    let limit = |world: &mut PlayerWorld, who: ObjId, max: &str| {
        world.say(
            GOD,
            &format!("@holder-grant-fixture limit #{} {} {max}", who.0, org.0),
        )
    };
    // The listing's price is 100.
    limit(world, member, "99")?;
    check!(
        crate::test_checks::refused(prepare_public(
            world,
            "market",
            "quote_derived_mind",
            1,
            member,
            base + 2,
            quote_params.clone()
        ))?,
        "a grant limited below the price refuses the Quote"
    );
    limit(world, member, "100")?;
    commit(
        world,
        "market",
        "quote_derived_mind",
        1,
        member,
        base + 3,
        quote_params,
    )?;
    let quote = rows(&disk(world), "market", "market_mind_entitlement_quote")
        .into_iter()
        .find(|v| v.get("publication") == Some(&json!(token(base + 3))))
        .ok_or("the member's quote")?;
    check_eq!(
        quote.get("buyer"),
        Some(&json!({"org": org, "acting": member})),
        "the holder is the organisation, the member acting"
    );
    // Doc 17h §3 M8f H1 (the lead's ruling): only the quoting member funds
    // an organisation's quote; another granted member is refused.
    limit(world, second, "none")?;
    check!(
        crate::test_checks::refused(prepare_public(
            world,
            "market",
            "purchase_derived_mind",
            1,
            second,
            base + 30,
            json!({"quote": token(base + 3)})
        ))?,
        "another member cannot fund the member's quote"
    );
    world.say(GOD, &format!("@holder-grant-fixture revoke #{}", second.0))?;
    world.say(GOD, &format!("@holder-grant-fixture revoke #{}", member.0))?;
    let fund = json!({"quote": token(base + 3)});
    check!(
        crate::test_checks::refused(prepare_public(
            world,
            "market",
            "purchase_derived_mind",
            1,
            member,
            base + 4,
            fund.clone()
        ))?,
        "a grant revoked after the Quote refuses the Fund"
    );
    limit(world, member, "100")?;
    commit(
        world,
        "market",
        "purchase_derived_mind",
        1,
        member,
        base + 5,
        fund,
    )?;
    let hold = store(world)?
        .snapshot()
        .holds_of(treasury)
        .next()
        .ok_or("the treasury's hold")?
        .clone();
    check_eq!(
        hold.placed_by,
        EconomicOwner::Organization(org),
        "the hold is placed by the organisation"
    );
    // S1: the quoting member loses its grant; the second member refunds,
    // under a grant limited at the price (doc 17h §3 M8f P2a: the Refund asks
    // with the price, so a limit below it refuses and one above it refunds).
    world.say(GOD, &format!("@holder-grant-fixture revoke #{}", member.0))?;
    limit(world, second, "99")?;
    check!(
        crate::test_checks::refused(prepare_public(
            world,
            "market",
            "refund_derived_mind",
            1,
            second,
            base + 31,
            json!({"operation": token(base + 5)})
        ))?,
        "a limit below the price refuses the Refund"
    );
    // A limit ABOVE the price refunds as well.
    limit(world, second, "150")?;
    commit(
        world,
        "market",
        "refund_derived_mind",
        1,
        second,
        base + 6,
        json!({"operation": token(base + 5)}),
    )?;
    check_eq!(
        store(world)?
            .snapshot()
            .account(treasury)
            .ok_or("treasury")?
            .held_amount,
        Amount::ZERO,
        "the organisation's escrow is released"
    );
    check_eq!(
        allocation(world, base + 5)?.phase(),
        PurchasePhase::Refunded {
            commit: token(base + 6)
        }
    );
    // A second purchase, completed by the quoting member.
    limit(world, member, "100")?;
    commit(
        world,
        "market",
        "quote_derived_mind",
        1,
        member,
        base + 7,
        json!({"listing": token(base + 1), "org": org.0}),
    )?;
    commit(
        world,
        "market",
        "purchase_derived_mind",
        1,
        member,
        base + 8,
        json!({"quote": token(base + 7)}),
    )?;
    let settle = json!({"operation": token(base + 8)});
    // Doc 17h §3 M8f P2a: the Grant asks the grant with the quoted price.
    limit(world, member, "99")?;
    check!(
        crate::test_checks::refused(prepare_public(
            world,
            "market",
            "settle_derived_mind",
            1,
            member,
            base + 9,
            settle.clone()
        ))?,
        "a grant limited below the price refuses the Grant"
    );
    limit(world, member, "100")?;
    commit(
        world,
        "market",
        "settle_derived_mind",
        1,
        member,
        base + 10,
        settle,
    )?;
    let a = allocation(world, base + 8)?;
    check_eq!(
        a.holder(),
        mush_economy::Holder::Organization {
            org,
            acting: member
        }
    );
    // S3: the second member, non-staff and never the buyer, renews it.
    let renewing = capture(world, &disk(world));
    let mut w = PlayerWorld::boot_renewals(f, &renewing)?;
    for actor in [GOD, BUYER, member, second] {
        w.engine.world.move_to(actor, ROOM)?;
    }
    let term = own_term(&w, a.licence())?;
    w.engine.world.set_now(term.window().term_end().get());
    let held = |w: &PlayerWorld| -> TestResult<Amount> {
        Ok(store(w)?
            .snapshot()
            .account(treasury)
            .ok_or("treasury")?
            .posted_balance)
    };
    let before = held(&w)?;
    // Doc 17h §3 M8f P2a: a limited grant renews. The Draft moves no money
    // and asks as a read, so a grant limited to 1 drafts; the Fund asks with
    // the renewal's price, so that limit refuses it and one at the price pays.
    let renewal_limit = |w: &mut PlayerWorld, max: &str| {
        w.say(
            GOD,
            &format!("@holder-grant-fixture limit #{} {} {max}", second.0, org.0),
        )
    };
    renewal_limit(&mut w, "1")?;
    commit(
        &mut w,
        "market",
        "draft_mind_renewal",
        1,
        second,
        base + 20,
        json!({"licence": a.licence().get()}),
    )?;
    let renewal: mush_roll::commerce::renewal::RenewalOperationV1 = serde_json::from_value(
        rows(&disk(&w), "roll", "roll_mind_renewal_operation")
            .into_iter()
            .find(|v| v.get("operation") == Some(&json!(token(base + 20))))
            .ok_or("the drafted renewal")?,
    )?;
    let price = renewal.binding().price_minor();
    check!(price > 1, "a renewal price above the limit of 1");
    check!(
        crate::test_checks::refused(prepare_public(
            &mut w,
            "market",
            "fund_mind_renewal",
            1,
            second,
            base + 32,
            json!({"operation": token(base + 20)})
        ))?,
        "a limit below the renewal price refuses the Fund"
    );
    renewal_limit(&mut w, &price.to_string())?;
    commit(
        &mut w,
        "market",
        "fund_mind_renewal",
        1,
        second,
        base + 21,
        json!({"operation": token(base + 20)}),
    )?;
    let now = w.engine.world.now();
    w.engine.world.set_now(now.checked_add(31).ok_or("time")?);
    commit(
        &mut w,
        "market",
        "settle_mind_renewal",
        1,
        second,
        base + 22,
        json!({"operation": token(base + 20)}),
    )?;
    check!(held(&w)? < before, "the treasury paid the renewal");
    let pointer: mush_roll::commerce::paid::PersonalPaidLicence = serde_json::from_value(
        rows(&disk(&w), "roll", "roll_mind_personal_paid_licence")
            .into_iter()
            .find(|v| v.get("id") == Some(&json!(a.licence())))
            .ok_or("the renewed pointer")?,
    )?;
    check_eq!(pointer.term().get(), 2, "renewed");
    check_eq!(
        pointer.holder(),
        mush_economy::Holder::Organization {
            org,
            acting: second
        },
        "the renewed pointer names the renewing character"
    );
    let row = rows(&disk(&w), "roll", "roll_mind_paid_holdings")
        .into_iter()
        .find(|v| v.get("licence") == Some(&json!(a.licence())))
        .ok_or("the holdings row")?;
    check_eq!(
        row.get("holder"),
        Some(&json!({"org": org, "acting": member})),
        "the row keeps the grant's acting"
    );
    PlayerWorld::boot_renewals(f, &capture(&w, &disk(&w)))?;
    Ok(())
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
    check_eq!(a.protocol(), EntitlementProtocol::OriginalV3);
    check_eq!(
        a.economics().grant().downstream().royalty_permille(),
        Some(150)
    );
    check_eq!(
        a.economics().grant().renewal().price(),
        Amount::from_minor(73)
    );
    check_eq!(a.terms().form(), PaidForm::OpenResale {});
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
        None,
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
    check!(crate::test_checks::refused(list(
        &mut world,
        owner,
        FIRST + 73
    ))?);
    check!(crate::test_checks::refused(grant(
        &mut world,
        FIRST + 72,
        FIRST + 74
    ))?); // Quote is only one second old.
    commit(
        &mut world,
        "market",
        "refund_derived_mind",
        1,
        GOD,
        FIRST + 75,
        json!({"operation":token(FIRST+72)}),
    )?;
    check_eq!(balances(&world)?, before);
    let a = allocation(&world, FIRST + 72)?;
    check_eq!(
        a.phase(),
        PurchasePhase::Refunded {
            commit: token(FIRST + 75)
        }
    );
    check!(
        !rows(&disk(&world), "roll", "roll_mind_personal_paid_licence")
            .iter()
            .any(|r| r.get("id") == Some(&json!(a.licence())))
    );
    check!(rows(&disk(&world), "roll", "roll_mind_paid_term_economics").is_empty());
    let snapshot = capture(&world, &disk(&world));
    let _ = PlayerWorld::boot_entitlements(f, &snapshot)?;
    Ok(())
}
#[track_caller]
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
            check_eq!(*audience, mush_core::Audience::Conn(conn));
            match response {
                MindResponse::FiledSource {
                    source_sha256,
                    lines,
                    ..
                } => {
                    check_eq!(Some(source_sha256.clone()), expected.map(|h| h.to_hex()));
                    check!(!lines.is_empty());
                }
                MindResponse::Error { message, .. } => {
                    check!(expected.is_none(), "paid source refused: {message}")
                }
                _ => return Err("unexpected paid source response".into()),
            }
        }
        if serde_json::to_string(event)?.contains("this.landed") {
            check!(expected.is_some());
            check_eq!(*audience, mush_core::Audience::Conn(conn));
        }
    }
    check_eq!(answers, 1);
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
            .all(|licence|licence.holder()!=mush_economy::Holder::Character(GOD) || licence.id()==a.licence()),"customer bought no ancestor licence");
        // New sortie under the customer's own Grace still works after the
        // issuer's selected parent has ended; no ancestor is re-resolved.
        world.say(GOD,"SIM TEAM red")?;world.say(GOD,"SIM AI 1")?;world.say(GOD,"SIM UNIT heavy_mk1")?;
        let selected=world.say(GOD,&format!("SIM MIND filed:{} PROFILE Default",a.licence().get()))?;
        assert!(PlayerWorld::private_text(&selected,GOD).contains("Your slot will carry"));
        world.say(GOD,"SIM START")?;assert_eq!(world.state()?,match_::MatchState::Boarding);
        // The lead character is read during Boarding (`lead_character_of`).
        assert_eq!(match_::lobby::lead_character_of(&world.engine.world,crate::arena::PartyId::RED),Some(GOD));
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
    title_fences(f, &filed, owner)?;
    original_v3(f, &filed)?;
    lapse_refund(f, &filed, owner)?;
    let (granted, allocation) = transactions(f, &filed, owner)?;
    story_phase("entitlement.org_buys", || async {
        org_buys(f, &granted, owner)
    })?
    .await?;
    story_phase("entitlement.org_members", || async {
        org_members(f, &granted, owner)
    })?
    .await?;
    story_phase("entitlement.procurement", || async {
        procurement::procurement_tick(f, &granted, owner)
    })?
    .await?;
    resale_routes::exercise(
        &f.accounts,
        &granted,
        allocation.licence(),
        GOD,
        false,
        "purchase.fixture",
        64_000_000,
        None,
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
