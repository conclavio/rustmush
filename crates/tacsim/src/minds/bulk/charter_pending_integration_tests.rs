#![cfg(test)]

//! Genuine pending personal title transfers over the actual paid/owner fixture.
//! The absent region event uses the parent's explicitly private event adapter;
//! all four claims owners, escrow, service selectors and Registry causes are real.
use super::*;
use mush_core::{AuthoritativeSubmit, DefinitionRef, OfferExecutionContext, OfferSubmission};
use mush_economy::managed_charter_transfer::CharterPendingHistory;
use mush_roll::commerce::registry_revocation::RegistryRevocationPolicy;
use std::sync::atomic::{AtomicBool, Ordering};
const START: u128 = 52_000_000;
const OFFER: &str = "offer_mind_charter_transfer";
const FUND: &str = "fund_mind_charter_transfer";
const SETTLE: &str = "settle_mind_charter_transfer";
const REFUND: &str = "refund_mind_charter_transfer";
const PENDING: &str = "roll_mind_charter_pending";
const HEAD: &str = "roll_mind_charter_pending_head";
const CAUSE: &str = "roll_registry_term_revocations";
const SANCTION: &str = "roll_registry_revoked_terms";

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
            r.key().owner.as_str() == "roll"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == version
        })
        .cloned()
        .ok_or_else(|| "actual pending action".into())
}
fn prepare(
    w: &mut IcWorld,
    id: &str,
    version: u32,
    actor: ObjId,
    serial: u128,
    parameters: Value,
    key: Option<mush_core::IdempotencyKeyText>,
) -> TestResult<Box<Prepared>> {
    let action = exact(w, id, version)?;
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&action)
        .ok_or("actual signed pending offer")?;
    let invocation = request(serial, actor, "roll", parameters.clone())?;
    let key = key
        .map(|k| {
            ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: principal.clone(),
                    owner: OwnerId::new("roll").expect("fixed owner"),
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
    Ok(Box::new(Prepared {
        p,
        output,
        invocation,
        action,
        serial,
    }))
}
fn stage(w: &mut IcWorld, prepared: Box<Prepared>) -> TestResult {
    let Prepared {
        p,
        output,
        invocation: inv,
        action,
        serial,
    } = *prepared;
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
        .ok_or("actual pending custody")?;
    let (bundle, install) = staged.into_parts();
    retain_writes(&mut w.disk, bundle.writes())?;
    w.engine.install_composed_action(install)?;
    Ok(())
}
fn land(w: &mut IcWorld, p: Box<Prepared>) -> TestResult {
    stage(w, p)?;
    take_install(w)
}
fn start_branch(f: &PurchaseFixture, template: &IcWorld, saved: &Durable) -> TestResult<IcWorld> {
    let mut w = IcWorld::boot_options(
        f,
        saved,
        template.base,
        template.field,
        template.hulls,
        true,
        true,
        None,
    )?;
    w.engine.set_submission_runtime(Arc::new(Runtime))?;
    Ok(w)
}
fn charter(w: &IcWorld) -> TestResult<ObjId> {
    let value = one(rows(&w.disk, "roll", "roll_mind_initial_filing"))?;
    let id: mush_roll::commerce::CharterId =
        serde_json::from_value(value.get("charter").cloned().ok_or("charter")?)?;
    Ok(id.object())
}
fn title(w: &IcWorld) -> TestResult<EconomicOwner> {
    Ok(w.engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .snapshot()
        .asset(charter(w)?)
        .ok_or("title")?
        .title_owner
        .clone())
}
fn history(w: &IcWorld, funding: u128) -> TestResult<CharterPendingHistory> {
    Ok(w.engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .pending_charter_history(token(funding))?
        .clone())
}
fn balance(w: &IcWorld, id: WalletId) -> TestResult<(Amount, Amount)> {
    let a = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?
        .snapshot()
        .account(id)
        .ok_or("wallet")?;
    Ok((a.posted_balance, a.held_amount))
}
fn buyer_balance(w: &IcWorld) -> TestResult<(Amount, Amount)> {
    let (_, _, payer) = charter_transfer_fixture::consideration(&w.engine, charter(w)?, BUYER)?;
    balance(w, payer.id)
}
fn paid_rows(w: &IcWorld) -> Vec<Value> {
    [
        "roll_mind_personal_paid_licence",
        "roll_mind_paid_term",
        "roll_mind_paid_purchase",
    ]
    .into_iter()
    .flat_map(|family| rows(&w.disk, "roll", family))
    .collect()
}
fn owner_claim(w: &mut IcWorld, serial: u128) -> TestResult<Arc<IcHostTicket>> {
    let hull = w.hulls[0];
    w.command(&format!("@asset/transfer #{}=#{}", hull.0, GOD.0))?;
    let ticket = Arc::new(w.ticket(GOD, hull, 1)?);
    w.land(&ticket, serial, hull, false)?;
    Ok(ticket)
}
fn fund(w: &mut IcWorld, serial: u128) -> TestResult<(WalletId, WalletId)> {
    let charter = charter(w)?;
    let (seller, payout, payer) =
        charter_transfer_fixture::consideration(&w.engine, charter, BUYER)?;
    assert_eq!(seller, GOD);
    let before = w.snapshot();
    let p = prepare(
        w,
        OFFER,
        2,
        GOD,
        serial,
        charter_transfer_fixture::offer_params(charter, BUYER, payout.id),
        None,
    )?;
    assert!(w.disk == before.disk, "offer preparation published");
    land(w, p)?;
    let params = json!({"offer":token(serial),"payer":payer.id.0.to_string()});
    assert!(prepare(w, FUND, 1, GOD, serial + 1, params.clone(), None).is_err());
    let money = balance(w, payer.id)?;
    let p = prepare(w, FUND, 1, BUYER, serial + 1, params, None)?;
    assert_eq!(balance(w, payer.id)?, money);
    assert_eq!(title(w)?, EconomicOwner::Object(GOD));
    stage(w, p)?;
    assert_eq!(balance(w, payer.id)?, money);
    take_install(w)?;
    let native = history(w, serial + 1)?;
    assert!(native.terminal().is_none());
    assert_eq!(balance(w, payer.id)?.0, money.0);
    assert_eq!(
        balance(w, payer.id)?.1,
        money.1.checked_add(Amount::from_minor(1)).ok_or("held")?
    );
    assert_eq!(title(w)?, EconomicOwner::Object(GOD));
    // Staff and ordinary callers cannot turn a terminal action into service authority.
    assert!(prepare(w, REFUND, 1, GOD, serial + 2, json!({}), None).is_err());
    assert!(
        prepare(
            w,
            FUND,
            1,
            BUYER,
            serial + 2,
            json!({"offer":token(serial),"payer":payer.id.0.to_string()}),
            None
        )
        .is_err()
    );
    assert!(
        prepare(
            w,
            OFFER,
            2,
            GOD,
            serial + 2,
            charter_transfer_fixture::offer_params(charter, BUYER, payout.id),
            None
        )
        .is_err()
    );
    Ok((payout.id, payer.id))
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
    let (request, proof) = value.ok_or("actual pending service selector")?.into_parts();
    assert_eq!(request.actor, GOD);
    assert_eq!(request.action.key().id.as_str(), id);
    assert_eq!(request.parameters, json!({}));
    let proof = *proof
        .ok_or("actual observation")?
        .downcast::<Arc<AtomicBool>>()
        .map_err(|_| "observation type")?;
    Ok((request, proof))
}
fn terminal(
    w: &mut IcWorld,
    request: &AuthoritativeSubmit,
    serial: u128,
) -> TestResult<Box<Prepared>> {
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
fn close(w: &mut IcWorld, open: u128, serial: u128) -> TestResult {
    let event = ic_sortie::fixture_close(&w.engine.world, GOD, token(open), w.base)?;
    let ticket = w.host.enqueue_close(&event)?;
    w.land(&ticket, serial, w.hulls[0], true)
}

pub(super) fn exercise(f: &PurchaseFixture, template: &IcWorld, seeded: &Durable) -> TestResult {
    eprintln!("M7_STORY phase=charter_pending event=begin");
    offer_and_custody(f, template, seeded)?;
    claims_and_unknown(f, template, seeded)?;
    deadline(f, template, seeded)?;
    refunds(f, template, seeded)?;
    compound(f, template, seeded)?;
    eprintln!("M7_STORY phase=charter_pending event=complete");
    Ok(())
}
fn offer_and_custody(f: &PurchaseFixture, template: &IcWorld, seeded: &Durable) -> TestResult {
    assert!(
        exact(template, OFFER, 2).is_err(),
        "default exposed pending offers"
    );
    let mut w = start_branch(f, template, seeded)?;
    let charter = charter(&w)?;
    let (_, payout, payer) = charter_transfer_fixture::consideration(&w.engine, charter, BUYER)?;
    let money = balance(&w, payer.id)?;
    let offered = prepare(
        &mut w,
        OFFER,
        2,
        GOD,
        START + 500,
        charter_transfer_fixture::offer_params(charter, BUYER, payout.id),
        None,
    )?;
    land(&mut w, offered)?;
    for _ in 0..3 {
        pulse(&mut w)?;
    }
    assert!(rows(&w.disk, "roll", PENDING).is_empty());
    assert_eq!(balance(&w, payer.id)?, money);
    assert!(
        w.engine.take_observed_authoritative_submit().is_none(),
        "seller offer automatically accepted"
    );
    let funded = prepare(
        &mut w,
        FUND,
        1,
        BUYER,
        START + 501,
        json!({"offer":token(START+500),"payer":payer.id.0.to_string()}),
        None,
    )?;
    land(&mut w, funded)?;
    assert!(
        IcWorld::boot(f, &w.snapshot(), w.base, w.field, w.hulls, true).is_err(),
        "disabled composition accepted retained pending escrow"
    );
    let (request, _) = next(&mut w, SETTLE)?;
    let settled = terminal(&mut w, &request, START + 502)?;
    land(&mut w, settled)?;
    assert_eq!(title(&w)?, EconomicOwner::Object(BUYER));
    w.reboot(f, &w.snapshot())?;

    // A real owned claim closing races Fund. Core admits one pending commit,
    // so neither can land under the other's custody: a claim close cannot
    // land under a prepared Fund, a Fund cannot prepare under a staged close,
    // and once the actual close lands Fund is refused by the closed claim.
    for after_stage in [false, true] {
        let mut w = start_branch(f, template, seeded)?;
        owner_claim(&mut w, START + 510)?;
        let charter = self::charter(&w)?;
        let (_, payout, payer) =
            charter_transfer_fixture::consideration(&w.engine, charter, BUYER)?;
        let offered = prepare(
            &mut w,
            OFFER,
            2,
            GOD,
            START + 511,
            charter_transfer_fixture::offer_params(charter, BUYER, payout.id),
            None,
        )?;
        land(&mut w, offered)?;
        let fund_params = json!({"offer":token(START+511),"payer":payer.id.0.to_string()});
        if after_stage {
            let event = ic_sortie::fixture_close(&w.engine.world, GOD, token(START + 510), w.base)?;
            let ticket = w.host.enqueue_close(&event)?;
            let (prepared, output, inv) = w.prepare(&ticket, START + 513)?;
            w.stage(&ticket, START + 513, prepared, output, &inv)?;
            let (bundle, installation) = w
                .engine
                .take_staged_action()?
                .ok_or("actual close custody")?
                .into_parts();
            assert!(
                prepare(
                    &mut w,
                    FUND,
                    1,
                    BUYER,
                    START + 512,
                    fund_params.clone(),
                    None
                )
                .is_err(),
                "Fund prepared under a staged claim close"
            );
            // This is an actual prepared four-owner close image, not a patched
            // claim counter.
            retain_writes(&mut w.disk, bundle.writes())?;
            w.engine.install_composed_action(installation)?;
            let refused = match prepare(&mut w, FUND, 1, BUYER, START + 514, fund_params, None) {
                Err(_) => true,
                Ok(p) => stage(&mut w, p).is_err(),
            };
            assert!(refused, "Fund accepted after the claim closed");
        } else {
            let p = prepare(&mut w, FUND, 1, BUYER, START + 512, fund_params, None)?;
            assert!(
                close(&mut w, START + 510, START + 513).is_err(),
                "claim close landed under a prepared Fund"
            );
            drop(p);
        }
        assert!(rows(&w.disk, "roll", PENDING).is_empty());
        assert_eq!(balance(&w, payer.id)?.1, Amount::from_minor(0));
    }
    Ok(())
}
fn claims_and_unknown(f: &PurchaseFixture, template: &IcWorld, seeded: &Durable) -> TestResult {
    let mut w = start_branch(f, template, seeded)?;
    owner_claim(&mut w, START)?;
    let paid = paid_rows(&w);
    let before = buyer_balance(&w)?;
    let (_, payer) = fund(&mut w, START + 10)?;
    let accepted = w.snapshot();
    let mut cold = w.reboot(f, &accepted)?;
    cold.engine.set_submission_runtime(Arc::new(Runtime))?;
    assert!(cold.held(cold.hulls[0])?);
    assert!(cold.host.resume(&cold.engine.world, token(START)).is_ok());
    // A new owner sortie on a second correctly titled hull is refused by pending state.
    cold.command(&format!("@asset/transfer #{}=#{}", cold.hulls[1].0, GOD.0))?;
    let forbidden = cold.ticket(GOD, cold.hulls[1], 1)?;
    assert!(cold.prepare(&forbidden, START + 12).is_err());
    for _ in 0..3 {
        pulse(&mut cold)?;
    }
    assert!(cold.engine.take_observed_authoritative_submit().is_none());
    close(&mut cold, START, START + 13)?;
    let (request, proof) = next(&mut cold, SETTLE)?;
    for _ in 0..3 {
        pulse(&mut cold)?;
    }
    assert!(
        cold.engine.take_observed_authoritative_submit().is_none(),
        "Unknown replaced the exact intent"
    );
    proof.store(true, Ordering::Release); // consumer observation fixture, not server proof
    let (retry, _) = next(&mut cold, SETTLE)?;
    assert_eq!(request.key, retry.key);
    let prepared = terminal(&mut cold, &retry, START + 14)?;
    let disk = cold.disk.clone();
    assert_eq!(title(&cold)?, EconomicOwner::Object(GOD));
    stage(&mut cold, prepared)?;
    let taken = cold
        .engine
        .take_staged_action()?
        .ok_or("terminal custody")?;
    let (bundle, installation) = taken.into_parts();
    let mut crash = cold.snapshot();
    retain_writes(&mut crash.disk, bundle.writes())?;
    let recovered = cold.reboot(f, &crash)?;
    assert_eq!(title(&recovered)?, EconomicOwner::Object(BUYER));
    assert!(cold.disk == disk, "pre-install mutated persistence fixture");
    retain_writes(&mut cold.disk, bundle.writes())?;
    cold.engine.install_composed_action(installation)?;
    assert_eq!(title(&cold)?, EconomicOwner::Object(BUYER));
    assert_eq!(
        balance(&cold, payer)?.0,
        before
            .0
            .checked_sub(Amount::from_minor(1))
            .ok_or("payment")?
    );
    assert_eq!(balance(&cold, payer)?.1, before.1);
    assert!(
        paid_rows(&cold) == paid,
        "charter transfer rewrote paid entitlement"
    );
    for mode in 0..4 {
        let mut image = cold.snapshot();
        let family = match mode {
            0 => PENDING,
            1 => HEAD,
            2 => "roll_mind_charter_transfers",
            _ => "roll_mind_charter_ownership",
        };
        let key = image
            .disk
            .iter()
            .find(|(_, v)| v.pointer("/family/id") == Some(&json!(family)))
            .map(|(k, _)| k.clone())
            .ok_or("actual paired row")?;
        image.disk.remove(&key);
        assert!(
            cold.reboot(f, &image).is_err(),
            "cold pending history lost a paired row"
        );
    }
    Ok(())
}
fn deadline(f: &PurchaseFixture, template: &IcWorld, seeded: &Durable) -> TestResult {
    let mut w = start_branch(f, template, seeded)?;
    owner_claim(&mut w, START + 100)?;
    fund(&mut w, START + 110)?;
    let h = history(&w, START + 111)?;
    let until = h.consent().deadline.at;
    w.engine.world.set_now(until);
    let (request, _) = next(&mut w, SETTLE)?;
    let p = terminal(&mut w, &request, START + 114)?;
    land(&mut w, p)?;
    assert_eq!(title(&w)?, EconomicOwner::Object(BUYER));
    assert!(w.held(w.hulls[0])?);
    assert!(w.host.resume(&w.engine.world, token(START + 100)).is_ok());
    let mut cold = w.reboot(f, &w.snapshot())?;
    assert!(
        cold.host
            .resume(&cold.engine.world, token(START + 100))
            .is_ok()
    );
    close(&mut cold, START + 100, START + 115)?;
    let new = cold.ticket(BUYER, cold.hulls[1], 1)?;
    cold.land(&new, START + 116, cold.hulls[1], false)?;
    let actual = rows(&cold.disk, "roll", "roll_mind_ic_claim")
        .into_iter()
        .find(|v| v.pointer("/open/operation") == Some(&json!(token(START + 116))))
        .ok_or("new owner claim")?;
    assert_eq!(
        actual.pointer("/open/charter_ownership/generation"),
        Some(&json!(2))
    );
    Ok(())
}
fn refunds(f: &PurchaseFixture, template: &IcWorld, seeded: &Durable) -> TestResult {
    for clock in [false, true] {
        let serial = START + if clock { 300 } else { 200 };
        let mut w = start_branch(f, template, seeded)?;
        owner_claim(&mut w, serial)?;
        let money = buyer_balance(&w)?;
        let (_, payer) = fund(&mut w, serial + 10)?;
        if clock {
            let epoch = w.engine.world.next_world_timeline_epoch(1)?;
            w.engine.world.append_world_timeline_epoch(epoch)?;
        } else {
            w.engine
                .world
                .get_mut(BUYER)
                .ok_or("buyer")?
                .flags
                .insert(mush_core::Flags::GOING);
            let _ = w.engine.world.take_dirty();
        }
        let (request, _) = next(&mut w, REFUND)?;
        let p = terminal(&mut w, &request, serial + 14)?;
        land(&mut w, p)?;
        assert_eq!(title(&w)?, EconomicOwner::Object(GOD));
        assert_eq!(balance(&w, payer)?, money);
        assert!(w.held(w.hulls[0])?);
        let image = w.snapshot();
        let cold = w.reboot(f, &image)?;
        assert!(history(&cold, serial + 11)?.terminal().is_some());
        if !clock {
            w.engine
                .world
                .get_mut(BUYER)
                .ok_or("buyer")?
                .flags
                .remove(mush_core::Flags::GOING);
            let _ = w.engine.world.take_dirty();
            assert!(
                prepare(
                    &mut w,
                    FUND,
                    1,
                    BUYER,
                    serial + 15,
                    json!({"offer":token(serial+10),"payer":payer.0.to_string()}),
                    None
                )
                .is_err(),
                "refunded offer was reused"
            );
            fund(&mut w, serial + 20)?; // explicit fresh offer and named buyer, never automatic acceptance
        }
    }
    Ok(())
}
fn compound(f: &PurchaseFixture, template: &IcWorld, seeded: &Durable) -> TestResult {
    let mut setup = start_branch(f, template, seeded)?;
    setup.command("@org/create PendingRegistry")?;
    setup.command("@org/scope create PendingRegistry=revocation")?;
    let orgs = setup
        .engine
        .world
        .module_state::<mush_org::OrgStore>()
        .ok_or("Org")?
        .state();
    let organization = orgs
        .orgs()
        .find(|o| o.name.as_str() == "PendingRegistry")
        .ok_or("Registry")?
        .id;
    let scope = orgs
        .scopes_in(organization)
        .find(|s| s.name.as_str() == "revocation")
        .ok_or("scope")?
        .id;
    setup.command(&format!(
        "@org/grant #{}=PendingRegistry roll.registry.revoke scope={}",
        BUYER.0, scope.0
    ))?;
    let policy = RegistryRevocationPolicy::new(issuer()?, organization, scope, vec![BUYER])?;
    let mut w = IcWorld::boot_options(
        f,
        &setup.snapshot(),
        setup.base,
        setup.field,
        setup.hulls,
        true,
        true,
        Some(policy),
    )?;
    owner_claim(&mut w, START + 400)?;
    let money = buyer_balance(&w)?;
    let (_, payer) = fund(&mut w, START + 410)?;
    let before = w.snapshot();
    let p = prepare(
        &mut w,
        "registry_term_revocation",
        1,
        BUYER,
        START + 414,
        json!({"licence":"1"}),
        None,
    )?;
    assert!(rows(&w.disk, "roll", CAUSE).is_empty());
    assert!(history(&w, START + 411)?.terminal().is_none());
    stage(&mut w, p)?;
    take_install(&mut w)?;
    assert_eq!(rows(&w.disk, "roll", CAUSE).len(), 1);
    assert_eq!(rows(&w.disk, "roll", SANCTION).len(), 1);
    let h = history(&w, START + 411)?;
    assert_eq!(
        h.terminal().ok_or("real refund")?.decision().operation,
        token(START + 414)
    );
    assert_eq!(balance(&w, payer)?, money);
    assert_eq!(title(&w)?, EconomicOwner::Object(GOD));
    assert!(w.held(w.hulls[0])?);
    assert!(w.host.resume(&w.engine.world, token(START + 400)).is_ok());
    let cold = w.reboot(f, &w.snapshot())?;
    assert!(history(&cold, START + 411)?.terminal().is_some());
    let mut corrupt = w.snapshot();
    corrupt
        .disk
        .retain(|_, v| v.pointer("/family/id") != Some(&json!(SANCTION)));
    assert!(
        w.reboot(f, &corrupt).is_err(),
        "refund lost its actual Registry sanction"
    );
    // The same compound preparation cannot cross a real authorization revocation.
    for phase in [0, 1] {
        let mut branch = w.reboot(f, &before)?;
        let p = prepare(
            &mut branch,
            "registry_term_revocation",
            1,
            BUYER,
            START + 415,
            json!({"licence":"1"}),
            None,
        )?;
        if phase == 0 {
            branch.command("@org/status PendingRegistry=suspended")?;
            assert!(stage(&mut branch, p).is_err());
        } else {
            stage(&mut branch, p)?;
            branch.command("@org/status PendingRegistry=suspended")?;
            assert!(branch.engine.take_staged_action().is_err());
        }
        assert!(history(&branch, START + 411)?.terminal().is_none());
        assert!(rows(&branch.disk, "roll", CAUSE).is_empty());
    }
    Ok(())
}
