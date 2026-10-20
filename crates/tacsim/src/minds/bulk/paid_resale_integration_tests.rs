//! Real signed resale over an acknowledged personal purchase and actual IC owners.
//! Only the existing private absent-region event boundary is used for IC claims;
//! all payment, holder changes, retention, native history and recovery are real.
use super::*;
use mush_core::{AuthoritativeSubmit, DefinitionRef, OfferExecutionContext, OfferSubmission};
use mush_roll::commerce::paid::PersonalPaidLicence;
use std::sync::atomic::{AtomicBool, Ordering};
const START: u128 = 62_000_000;
const OFFER: &str = "offer_paid_mind_resale";
const ACCEPT: &str = "accept_paid_mind_resale";
const FUND: &str = "fund_paid_mind_resale";
const SETTLE: &str = "settle_paid_mind_resale";
const REFUND: &str = "refund_paid_mind_resale";
const POINTER: &str = "roll_mind_personal_paid_licence";
const HOLDER: &str = "roll_mind_paid_holders";
const ROLL: &str = "roll_mind_paid_resales";
const MARKET: &str = "market_mind_paid_resales";
const RECEIPT: &str = "market_mind_paid_resale_receipts";
struct Prepared {
    p: PreparedComposedTxn,
    output: PreparedAuthoritativeAction,
    invocation: ActionInvocation,
    action: DefinitionRef,
    serial: u128,
}
fn exact(w: &IcWorld, id: &str, version: u32) -> TestResult<DefinitionRef> {
    w.engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|d| d.reference())
        .find(|r| {
            r.key().owner.as_str()
                == if id == "registry_term_revocation" {
                    "roll"
                } else {
                    "market"
                }
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == version
        })
        .cloned()
        .ok_or_else(|| "actual resale action".into())
}
fn prepare(
    w: &mut IcWorld,
    id: &str,
    version: u32,
    actor: ObjId,
    serial: u128,
    parameters: Value,
    key: Option<mush_core::IdempotencyKeyText>,
) -> TestResult<Prepared> {
    let action = exact(w, id, version)?;
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&action)
        .ok_or("actual signed resale offer")?;
    let invocation = request(
        serial,
        actor,
        action.key().owner.as_str(),
        parameters.clone(),
    )?;
    let key = key
        .map(|k| {
            ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: principal.clone(),
                    owner: action.key().owner.clone(),
                    namespace: CanonicalId::new(id).expect("fixed action"),
                },
                1,
                IdempotencyKey::Caller(k),
            )
        })
        .unwrap_or_else(|| invocation.idempotency().clone());
    let resolved = w.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: invocation.request(),
            principal,
            actor,
            idempotency: key,
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
    let (actual, resolved, _) = resolved.into_authoritative_parts();
    assert_eq!(actual, action);
    let invocation = resolved.invocation().clone();
    let (p, output) = w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            token(serial),
        )?;
    Ok(Prepared {
        p,
        output,
        invocation,
        action,
        serial,
    })
}
fn stage(w: &mut IcWorld, prepared: Prepared) -> TestResult {
    let Prepared {
        p,
        output,
        invocation: inv,
        action,
        serial,
    } = prepared;
    let (outcome, resolution, mut definitions, mut witnesses, mut durable_outbox, semantic_events) =
        output.into_parts();
    definitions.extend([action.clone(), outcome.schema().clone()]);
    definitions.sort();
    definitions.dedup();
    if witnesses.is_empty() {
        witnesses.push(inv.principal().clone());
    }
    witnesses.sort();
    witnesses.dedup();
    durable_outbox.sort_by_key(|o| o.ordinal());
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(serial),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([31; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([32; 32]),
        action,
        provider: p.owner().clone(),
        cause: PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: p.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token(serial))),
        semantic_events,
        eligible_witnesses: witnesses,
        durable_outbox,
    })?;
    let guards = w
        .engine
        .registry
        .server_guard_set_builder(RevisionSet::default())?
        .finish()?;
    w.engine.stage_authoritative_action(p, guards, bundle)?;
    Ok(())
}
fn take_install(w: &mut IcWorld) -> TestResult {
    let staged = w
        .engine
        .take_staged_action()?
        .ok_or("actual resale custody")?;
    let (bundle, install) = staged.into_parts();
    retain_writes(&mut w.disk, bundle.writes())?;
    w.engine.install_composed_action(install)?;
    Ok(())
}
fn land(w: &mut IcWorld, p: Prepared) -> TestResult {
    stage(w, p)?;
    take_install(w)
}

fn branch(f: &PurchaseFixture, template: &IcWorld, saved: &Durable) -> TestResult<IcWorld> {
    let mut w = IcWorld::boot_all(
        f,
        saved,
        template.base,
        template.field,
        template.hulls,
        true,
        false,
        true,
        None,
    )?;
    w.engine.set_submission_runtime(Arc::new(Runtime))?;
    // A native cash mint backs the new buyer's escrow; credit is not a hold.
    let faucet = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .snapshot()
        .accounts()
        .find(|a| {
            a.owner
                == EconomicOwner::System(
                    SystemPrincipalId::new("purchase.fixture").expect("fixed owner"),
                )
        })
        .ok_or("actual prior funding system")?
        .id;
    w.command(&format!("@econ/mint {faucet}=0 1000"))?;
    Ok(w)
}
fn pointer(w: &IcWorld) -> TestResult<PersonalPaidLicence> {
    let value = one(rows(&w.disk, "roll", POINTER))?;
    Ok(serde_json::from_value(value)?)
}
fn receipt_at_acceptance(
    w: &IcWorld,
    acceptance: u128,
    terminal: u128,
) -> TestResult<(String, String)> {
    let (key, document) = w
        .disk
        .iter()
        .find(|((owner, _), document)| {
            owner == "market"
                && document["family"]["id"] == RECEIPT
                && document["value"]["acceptance"] == json!(token(acceptance))
        })
        .ok_or("actual terminal receipt")?;
    assert_eq!(document["key"], json!(token(acceptance)));
    assert_eq!(document["value"]["operation"], json!(token(terminal)));
    assert!(key.1.ends_with(&format!("/{}", token(acceptance).to_hex())));
    Ok(key.clone())
}
fn refuse_terminal_receipt_key(
    f: &PurchaseFixture,
    w: &IcWorld,
    acceptance: u128,
    terminal: u128,
) -> TestResult {
    let key = receipt_at_acceptance(w, acceptance, terminal)?;
    assert_ne!(acceptance, terminal);
    let mut crossed = w.snapshot();
    let mut document = crossed.disk.remove(&key).ok_or("actual receipt")?;
    let prefix = key
        .1
        .strip_suffix(&token(acceptance).to_hex())
        .ok_or("actual receipt key suffix")?;
    document["key"] = json!(token(terminal));
    crossed.disk.insert(
        (key.0, format!("{prefix}{}", token(terminal).to_hex())),
        document,
    );
    assert!(
        w.reboot(f, &crossed).is_err(),
        "terminal-keyed receipt escaped the reserved acceptance"
    );
    Ok(())
}
fn history(
    w: &IcWorld,
    acceptance: u128,
) -> TestResult<mush_economy::managed_paid_resale::PaidResaleHistoryV1> {
    Ok(w.engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .paid_resale_history(token(acceptance))?
        .clone())
}
fn balance(w: &IcWorld, id: WalletId) -> TestResult<(Amount, Amount)> {
    let e = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?;
    let s = e.snapshot();
    let a = s.account(id).ok_or("actual account")?;
    Ok((a.posted_balance, a.held_amount))
}
fn offer(w: &mut IcWorld, serial: u128, seller: ObjId, buyer: ObjId, account: u64) -> TestResult {
    let id = pointer(w)?.id();
    let p = prepare(
        w,
        OFFER,
        1,
        seller,
        serial,
        json!({"licence":id.get(),"buyer":buyer.0,"price":100,"seller_account":account}),
        None,
    )?;
    land(w, p)
}
fn buy(
    w: &mut IcWorld,
    serial: u128,
    offer: u128,
    kind: &str,
    buyer: ObjId,
    payer: u64,
) -> TestResult {
    let p = prepare(
        w,
        kind,
        1,
        buyer,
        serial,
        json!({"offer":token(offer),"payer":payer}),
        None,
    )?;
    land(w, p)
}
fn immediate(f: &PurchaseFixture, template: &IcWorld, saved: &Durable) -> TestResult {
    let mut w = branch(f, template, saved)?;
    let before = pointer(&w)?;
    let term = rows(&w.disk, "roll", "roll_mind_paid_term");
    let allocation = rows(&w.disk, "roll", "roll_mind_paid_purchase");
    let seller = balance(&w, WalletId(2))?;
    let buyer = balance(&w, WalletId(0))?;
    offer(&mut w, START, BUYER, GOD, 2)?;
    assert_eq!(pointer(&w)?, before, "offer changed the holder");
    assert!(
        prepare(
            &mut w,
            ACCEPT,
            1,
            BUYER,
            START + 1,
            json!({"offer":token(START),"payer":2}),
            None
        )
        .is_err()
    );
    assert!(
        prepare(
            &mut w,
            FUND,
            1,
            GOD,
            START + 2,
            json!({"offer":token(START),"payer":0}),
            None
        )
        .is_err(),
        "zero claims admitted pending custody"
    );
    let p = prepare(
        &mut w,
        ACCEPT,
        1,
        GOD,
        START + 3,
        json!({"offer":token(START),"payer":0}),
        None,
    )?;
    stage(&mut w, p)?;
    let staged = w
        .engine
        .take_staged_action()?
        .ok_or("actual resale custody")?;
    let (bundle, install) = staged.into_parts();
    assert_eq!(pointer(&w)?, before, "pre-ACK holder changed");
    let mut crash = w.snapshot();
    retain_writes(&mut crash.disk, bundle.writes())?;
    let mut cold = w.reboot(f, &crash)?;
    let changed = pointer(&cold)?;
    assert_eq!(changed.holder(), GOD);
    assert_eq!(changed.id(), before.id());
    assert_eq!(changed.term(), before.term());
    assert_eq!(changed.operation(), before.operation());
    assert_eq!(changed.ownership().ok_or("captured holder")?.generation, 2);
    assert_eq!(rows(&cold.disk, "roll", "roll_mind_paid_term"), term);
    assert_eq!(
        rows(&cold.disk, "roll", "roll_mind_paid_purchase"),
        allocation
    );
    assert_eq!(
        history(&cold, START + 3)?.outcome(),
        mush_economy::managed_paid_resale::PaidResaleOutcome::Settled
    );
    assert!(balance(&cold, WalletId(2))?.0 > seller.0);
    assert!(balance(&cold, WalletId(0))?.0 < buyer.0);
    retain_writes(&mut w.disk, bundle.writes())?;
    w.engine.install_composed_action(install)?;
    assert_eq!(pointer(&w)?, changed);
    assert_eq!(history(&w, START + 3)?, history(&cold, START + 3)?);
    receipt_at_acceptance(&w, START + 3, START + 3)?;
    assert!(
        prepare(
            &mut cold,
            ACCEPT,
            1,
            GOD,
            START + 4,
            json!({"offer":token(START),"payer":0}),
            None
        )
        .is_err(),
        "offer accepted twice"
    );
    assert!(
        cold.ticket(BUYER, cold.hulls[0], before.id().get())
            .and_then(|t| cold.prepare(&t, START + 5))
            .is_err(),
        "seller retained new-seal admission"
    );
    // Real title transfer of the hull is independent of the licence transfer.
    cold.command(&format!("@asset/transfer #{}=#{}", cold.hulls[0].0, GOD.0))?;
    let ticket = cold.ticket(GOD, cold.hulls[0], before.id().get())?;
    cold.land(&ticket, START + 6, cold.hulls[0], false)?;
    let claim = rows(&cold.disk, "roll", "roll_mind_ic_claim")
        .into_iter()
        .find(|v| v["open"]["operation"] == json!(token(START + 6)))
        .ok_or("actual transferred claim")?;
    assert_eq!(claim["format"], 3);
    assert_eq!(claim["open"]["paid_ownership"]["generation"], 2);
    // Cold completeness is tested against actual installed paired images.
    for (owner, family) in [
        ("roll", ROLL),
        ("roll", HOLDER),
        ("market", MARKET),
        ("market", RECEIPT),
    ] {
        let mut missing = crash.clone();
        let k = missing
            .disk
            .keys()
            .find(|(ns, key)| ns == owner && key.contains(family))
            .cloned()
            .ok_or("actual retained family")?;
        missing.disk.remove(&k);
        assert!(
            w.reboot(f, &missing).is_err(),
            "missing paired resale row accepted"
        );
    }
    Ok(())
}
fn pulse(w: &mut IcWorld) -> TestResult {
    let now = w.engine.world.now().checked_add(1).ok_or("clock")?;
    w.engine.world.set_now(now);
    for _ in 0..4 {
        w.engine.fire_due(
            now.checked_mul(1000).ok_or("clock")?,
            &mut Outbox::new(),
            &mut mush_core::Effects::new(),
        );
    }
    w.persist_setup()
}
fn next(w: &mut IcWorld, id: &str) -> TestResult<(Box<AuthoritativeSubmit>, Arc<AtomicBool>)> {
    let mut value = w.engine.take_observed_authoritative_submit();
    for _ in 0..8 {
        if value.is_some() {
            break;
        }
        pulse(w)?;
        value = w.engine.take_observed_authoritative_submit();
    }
    let (request, proof) = value.ok_or("actual resale service selector")?.into_parts();
    assert_eq!(request.actor, GOD);
    assert_eq!(request.action.key().id.as_str(), id);
    assert_eq!(request.parameters, json!({}));
    let proof = *proof
        .ok_or("actual observation")?
        .downcast::<Arc<AtomicBool>>()
        .map_err(|_| "observation type")?;
    Ok((request, proof))
}
fn terminal(w: &mut IcWorld, request: &AuthoritativeSubmit, serial: u128) -> TestResult<Prepared> {
    prepare(
        w,
        request.action.key().id.as_str(),
        1,
        request.actor,
        serial,
        request.parameters.clone(),
        Some(request.key.clone()),
    )
}

fn pending(f: &PurchaseFixture, template: &IcWorld, saved: &Durable, mode: u8) -> TestResult {
    let mut w = branch(f, template, saved)?;
    let before = pointer(&w)?;
    let hull = w.hulls[0];
    let n = START + 100 + u128::from(mode) * 100;
    let ticket = w.ticket(BUYER, hull, before.id().get())?;
    w.land(&ticket, n, hull, false)?;
    let claim = one(rows(&w.disk, "roll", "roll_mind_ic_claim"))?;
    assert_eq!(claim["format"], 1);
    offer(&mut w, n + 1, BUYER, GOD, 2)?;
    assert!(
        prepare(
            &mut w,
            ACCEPT,
            1,
            GOD,
            n + 2,
            json!({"offer":token(n+1),"payer":0}),
            None
        )
        .is_err(),
        "occupied licence admitted immediate sale"
    );
    let cash = balance(&w, WalletId(0))?;
    buy(&mut w, n + 3, n + 1, FUND, GOD, 0)?;
    assert_eq!(pointer(&w)?, before);
    assert_eq!(balance(&w, WalletId(0))?.0, cash.0);
    assert_eq!(
        balance(&w, WalletId(0))?.1,
        cash.1.checked_add(Amount::from_minor(100)).ok_or("held")?
    );
    assert!(
        prepare(&mut w, SETTLE, 1, GOD, n + 4, json!({}), None).is_err(),
        "direct service invocation gained authority"
    );
    assert!(
        w.ticket(BUYER, w.hulls[1], before.id().get())
            .and_then(|t| w.prepare(&t, n + 5))
            .is_err(),
        "pending seller gained a new seal"
    );
    let funded = w.snapshot();
    let mut w = w.reboot(f, &funded)?;
    w.engine.set_submission_runtime(Arc::new(Runtime))?;
    assert_eq!(pointer(&w)?, before);
    assert!(w.held(hull)?);
    let id = if mode == 2 {
        // The service actor stays live. A real account-policy command makes
        // the accepted payer unavailable under the native consent predicate.
        w.command("@account/approval 0=1 threshold:1")?;
        assert_eq!(
            w.engine
                .world
                .module_state::<mush_economy::EconomyStore>()
                .ok_or("Economy")?
                .paid_resale_availability(&w.engine.world, token(n + 3))?,
            mush_economy::managed_paid_resale::PaidResaleAvailability::RecipientUnavailable
        );
        REFUND
    } else {
        if mode == 0 {
            let event = ic_sortie::fixture_close(&w.engine.world, BUYER, token(n), w.base)?;
            let close = w.host.enqueue_close(&event)?;
            w.land(&close, n + 6, hull, true)?;
        } else {
            // The fixture WorldIC default is actual ratio1/offset0.
            let deadline = history(&w, n + 3)?.consent().deadline.at;
            w.engine.world.set_now(deadline);
        }
        SETTLE
    };
    let (selector, proof) = next(&mut w, id)?;
    pulse(&mut w)?;
    assert!(
        w.engine.take_observed_authoritative_submit().is_none(),
        "Unknown was resubmitted"
    );
    proof.store(true, Ordering::Release);
    let (retry, _) = next(&mut w, id)?;
    assert_eq!(selector.key, retry.key);
    assert_eq!(selector.parameters, retry.parameters);
    let p = terminal(&mut w, &retry, n + 7)?;
    land(&mut w, p)?;
    assert_eq!(balance(&w, WalletId(0))?.1, cash.1);
    if mode == 2 {
        assert_eq!(pointer(&w)?, before);
        assert_eq!(balance(&w, WalletId(0))?.0, cash.0);
        assert_eq!(
            history(&w, n + 3)?.outcome(),
            mush_economy::managed_paid_resale::PaidResaleOutcome::Refunded
        );
    } else {
        assert_eq!(pointer(&w)?.holder(), GOD);
        assert_eq!(
            history(&w, n + 3)?.outcome(),
            mush_economy::managed_paid_resale::PaidResaleOutcome::Settled
        );
        if mode == 1 {
            assert!(w.held(hull)?, "deadline erased an old captured warrant");
            assert_eq!(one(rows(&w.disk, "roll", "roll_mind_ic_claim"))?, claim);
            w.command(&format!("@asset/transfer #{}=#{}", w.hulls[1].0, GOD.0))?;
            assert!(
                w.ticket(GOD, w.hulls[1], before.id().get())
                    .and_then(|t| w.prepare(&t, n + 8))
                    .is_err(),
                "new buyer exceeded seats minus old held claims"
            );
        }
    }
    let cold = w.reboot(f, &w.snapshot())?;
    assert_eq!(pointer(&cold)?, pointer(&w)?);
    assert_eq!(history(&cold, n + 3)?, history(&w, n + 3)?);
    refuse_terminal_receipt_key(f, &cold, n + 3, n + 7)?;
    Ok(())
}

fn lapsed(f: &PurchaseFixture, template: &IcWorld, saved: &Durable) -> TestResult {
    let mut w = branch(f, template, saved)?;
    let original = pointer(&w)?;
    let term: mush_roll::commerce::paid::PaidTermRecord =
        serde_json::from_value(one(rows(&w.disk, "roll", "roll_mind_paid_term"))?)?;
    w.engine.world.set_now(
        term.window()
            .grace_end()
            .get()
            .checked_add(1)
            .ok_or("clock")?,
    );
    offer(&mut w, START + 500, BUYER, GOD, 2)?;
    buy(&mut w, START + 501, START + 500, ACCEPT, GOD, 0)?;
    assert_eq!(pointer(&w)?.holder(), GOD);
    assert_eq!(pointer(&w)?.term(), original.term());
    assert_eq!(
        one(rows(&w.disk, "roll", "roll_mind_paid_term"))?,
        serde_json::to_value(&term)?
    );
    w.command(&format!("@asset/transfer #{}=#{}", w.hulls[0].0, GOD.0))?;
    assert!(
        w.ticket(GOD, w.hulls[0], original.id().get())
            .and_then(|t| w.prepare(&t, START + 502))
            .is_err(),
        "lapsed transfer revived use"
    );
    assert_eq!(pointer(&w.reboot(f, &w.snapshot())?)?, pointer(&w)?);
    Ok(())
}
fn compound(f: &PurchaseFixture, template: &IcWorld, saved: &Durable) -> TestResult {
    let mut setup = branch(f, template, saved)?;
    setup.command("@org/create ResaleRegistry")?;
    setup.command("@org/scope create ResaleRegistry=revocation")?;
    let org = setup
        .engine
        .world
        .module_state::<mush_org::OrgStore>()
        .ok_or("Org")?
        .state();
    let organization = org
        .orgs()
        .find(|o| o.name.as_str() == "ResaleRegistry")
        .ok_or("Registry")?
        .id;
    let scope = org
        .scopes_in(organization)
        .find(|s| s.name.as_str() == "revocation")
        .ok_or("scope")?
        .id;
    setup.command(&format!(
        "@org/grant #{}=ResaleRegistry roll.registry.revoke scope={}",
        BUYER.0, scope.0
    ))?;
    let policy = mush_roll::commerce::registry_revocation::RegistryRevocationPolicy::new(
        issuer()?,
        organization,
        scope,
        vec![BUYER],
    )?;
    let mut w = IcWorld::boot_all(
        f,
        &setup.snapshot(),
        setup.base,
        setup.field,
        setup.hulls,
        true,
        false,
        true,
        Some(policy),
    )?;
    let before = pointer(&w)?;
    let hull = w.hulls[0];
    let money = balance(&w, WalletId(0))?;
    let ticket = w.ticket(BUYER, hull, before.id().get())?;
    w.land(&ticket, START + 600, hull, false)?;
    offer(&mut w, START + 601, BUYER, GOD, 2)?;
    buy(&mut w, START + 602, START + 601, FUND, GOD, 0)?;
    let funded = w.snapshot();
    let parameters = json!({"licence":before.id().get().to_string()});
    let p = prepare(
        &mut w,
        "registry_term_revocation",
        1,
        BUYER,
        START + 603,
        parameters.clone(),
        None,
    )?;
    assert!(history(&w, START + 602)?.terminal().is_none());
    assert!(rows(&w.disk, "roll", "roll_registry_term_revocations").is_empty());
    land(&mut w, p)?;
    assert_eq!(pointer(&w)?, before);
    assert_eq!(balance(&w, WalletId(0))?, money);
    refuse_terminal_receipt_key(f, &w, START + 602, START + 603)?;
    let h = history(&w, START + 602)?;
    assert_eq!(
        h.outcome(),
        mush_economy::managed_paid_resale::PaidResaleOutcome::Refunded
    );
    assert_eq!(
        h.terminal().ok_or("actual refund")?.decision().operation,
        token(START + 603)
    );
    assert_eq!(
        rows(&w.disk, "roll", "roll_registry_term_revocations").len(),
        1
    );
    assert_eq!(
        rows(&w.disk, "roll", "roll_registry_revoked_terms").len(),
        1
    );
    assert!(w.held(hull)?);
    assert!(w.host.resume(&w.engine.world, token(START + 600)).is_ok());
    let cold = w.reboot(f, &w.snapshot())?;
    assert_eq!(history(&cold, START + 602)?, h);
    let mut corrupt = w.snapshot();
    corrupt
        .disk
        .retain(|_, v| v.pointer("/family/id") != Some(&json!("roll_registry_revoked_terms")));
    assert!(
        w.reboot(f, &corrupt).is_err(),
        "compound refund lost its actual sanction"
    );
    for phase in [0, 1] {
        let mut branch = w.reboot(f, &funded)?;
        let p = prepare(
            &mut branch,
            "registry_term_revocation",
            1,
            BUYER,
            START + 604,
            parameters.clone(),
            None,
        )?;
        if phase == 0 {
            branch.command("@org/status ResaleRegistry=suspended")?;
            assert!(stage(&mut branch, p).is_err());
        } else {
            stage(&mut branch, p)?;
            branch.command("@org/status ResaleRegistry=suspended")?;
            assert!(branch.engine.take_staged_action().is_err());
        }
        assert!(history(&branch, START + 602)?.terminal().is_none());
        assert!(rows(&branch.disk, "roll", "roll_registry_term_revocations").is_empty());
    }
    Ok(())
}

pub(super) fn exercise(f: &PurchaseFixture, template: &IcWorld, saved: &Durable) -> TestResult {
    eprintln!("M7_STORY phase=paid_resale event=begin");
    immediate(f, template, saved)?;
    lapsed(f, template, saved)?;
    compound(f, template, saved)?;
    for mode in 0..3 {
        pending(f, template, saved, mode)?;
    }
    eprintln!("M7_STORY phase=paid_resale event=complete");
    Ok(())
}

#[path = "house_two_seat_integration_tests.rs"]
pub(super) mod two_seat;
