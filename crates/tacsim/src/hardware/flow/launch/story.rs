//! The real named source is saved, rated and filed through its actual services.
//! This in-process driver installs the exact acknowledged owner union. Its
//! observation adapter models no transport absence and grants no authority.
use super::*;
#[path = "story/pacing.rs"]
mod pacing;
use mush_core::{
    AuthoritativeSubmit, CanonicalId, IdempotencyKey, IdempotencyScope, Inbound,
    OfferExecutionContext, OfferSubmission, Outbox, PerceivedRef, RequestUid, ScopedIdempotencyKey,
};
use mush_roll::commerce::{
    LicenceId, TermWindow,
    paid::{PaidTermRecord, PersonalPaidLicence},
    rating::{RatingReceipt, RatingRef},
    records::OwnerTerm,
};
use std::time::{Duration, Instant};

struct Pending;
impl mush_core::submission_observation::SubmissionDispositionReader for Pending {
    fn no_intent(&self) -> bool {
        false
    }
}
struct Runtime;
impl mush_core::submission_observation::SubmissionRuntime for Runtime {
    fn observe(
        &self,
        _: &AuthoritativeSubmit,
    ) -> mush_core::submission_observation::SubmissionRuntimeState {
        mush_core::submission_observation::SubmissionRuntimeState::new(
            Arc::new(Pending),
            Box::new(()),
        )
    }
}
fn connected(r: &mut Rig) -> Result {
    r.world_mut()
        .engine_mut()
        .set_submission_runtime(Arc::new(Runtime))?;
    r.world_mut().engine_mut().step(
        Inbound::ConnUp {
            conn: 0,
            actor: GOD,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut Outbox::new(),
        &mut mush_core::Effects::new(),
    );
    r.flush();
    Ok(())
}
fn install_submit(r: &mut Rig, request: AuthoritativeSubmit) -> Result {
    check_eq!(request.actor, GOD);
    check!(request.targets.is_empty());
    check_eq!(request.subject, PerceivedRef::SelfActor);
    check_eq!(request.action.key().owner.as_str(), "roll");
    check!(request.action.key().id.as_str().starts_with("rating_"));
    check_eq!(
        request.parameters,
        json!({}),
        "actual private service selector"
    );
    r.serial += 1;
    let n = r.serial;
    let e = r.world_mut().engine_mut();
    e.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x46; 32])?,
        [],
    )?);
    let principal = mush_core::PrincipalId::Object { id: request.actor };
    let query = e.legal_action_query(principal.clone(), request.actor, request.subject.clone())?;
    let offers = e.project_legal_actions(&query)?;
    let offer = offers
        .projected_for_action(&request.action)
        .ok_or("actual Registry offer")?;
    let resolved = e.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: RequestUid::from_bytes(n.to_be_bytes()),
            principal: principal.clone(),
            actor: request.actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: request.action.key().owner.clone(),
                    namespace: CanonicalId::new(request.action.key().id.as_str())?,
                },
                1,
                IdempotencyKey::Caller(request.key),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: offer.offer(),
            subject: request.subject,
            targets: request.targets,
            parameters: request.parameters,
        },
    )?;
    let (action, resolved, guards) = resolved.into_authoritative_parts();
    check_eq!(action, request.action);
    let invocation = resolved.invocation().clone();
    let commit = driver::token(n);
    let prepared =
        e.prepare_registered_composed_resolved_authoritative_action(&action, &resolved, commit)?;
    r.install(driver::Planned {
        action,
        invocation,
        guards,
        prepared,
        commit,
    })?;
    Ok(())
}
fn pulse(r: &mut Rig) -> Result {
    if let Some(when) = r.world_mut().engine_mut().sched.next_due() {
        r.world_mut().fire_due(when);
    }
    r.flush();
    if let Some(submitted) = r
        .world_mut()
        .engine_mut()
        .take_observed_authoritative_submit()
    {
        check!(submitted.is_observed());
        let (request, _) = submitted.into_parts();
        install_submit(r, *request)?;
    }
    Ok(())
}
fn revision(r: &Rig, name: &str) -> Result<Option<mush_roll::Revision>> {
    let store = r
        .world()
        .world()
        .module_state::<mush_roll::RollStore>()
        .ok_or("Roll")?;
    let mind = store.mind(GOD, name)?;
    Ok(mind
        .latest()
        .and_then(|reference| store.revision(GOD, reference).ok())
        .cloned())
}
async fn certify(r: &mut Rig, name: &str, ordinal: u8) -> Result<(mush_roll::Revision, RatingRef)> {
    command(
        r.world_mut(),
        GOD,
        &format!("@mind/save {name}=actual named launch save"),
    )?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let revision = loop {
        pulse(r)?;
        if let Some(revision) = revision(r, name)? {
            break revision;
        }
        if Instant::now() >= deadline {
            return Err("actual named author service did not save".into());
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    let reference = revision.reference();
    // The previous rating's receipt can be read a pulse before the service
    // releases its finished job, and a second `@registry/rate` for GOD is
    // refused until it does; wait for the release rather than race it.
    let deadline = Instant::now() + Duration::from_secs(60);
    while crate::minds::bulk::registry_service::holds(r.world().world(), GOD) {
        if Instant::now() >= deadline {
            return Err("the previous Registry job was never released".into());
        }
        pulse(r)?;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let mut pacing = pacing::Progress::new(ordinal);
    command(
        r.world_mut(),
        GOD,
        &format!(
            "@registry/rate {}/{}",
            reference.key.mind, reference.key.number
        ),
    )?;
    let deadline = Instant::now() + Duration::from_secs(7200);
    loop {
        pulse(r)?;
        for wire in r.rows("rating_receipts") {
            let receipt: RatingReceipt = serde_json::from_value(wire)?;
            let rating = receipt.reference()?;
            if receipt.subject().revision()
                == mush_roll::commerce::RevisionIdentity::capture(&revision)?
            {
                pacing.sample(r, true);
                pacing.finish(true);
                return Ok((revision, rating));
            }
        }
        for wire in r.rows("rating_jobs") {
            let job: mush_roll::commerce::rating::RatingJob = serde_json::from_value(wire)?;
            use mush_roll::commerce::rating::RatingJobState;
            pacing.job(&job, &revision);
            if matches!(
                job.state(),
                RatingJobState::Failed { .. }
                    | RatingJobState::Cancelled {}
                    | RatingJobState::Interrupted {}
                    | RatingJobState::Nonconforming { .. }
            ) {
                pacing.sample(r, true);
                pacing.finish(false);
                return Err("actual named source did not certify".into());
            }
        }
        if Instant::now() >= deadline {
            pacing.sample(r, true);
            pacing.finish(false);
            return Err("actual named Registry service timed out".into());
        }
        pacing.sample(r, false);
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}
fn member(r: &mut Rig, present: bool) -> Result {
    let name = r.world().world().get(r.buyer).ok_or("buyer")?.name.clone();
    let line = if present {
        format!("@faction/add Republic={name}/level:1")
    } else {
        format!("@faction/remove Republic={name}")
    };
    command(r.world_mut(), GOD, &line)?;
    r.flush();
    Ok(())
}
fn one(r: &Rig, family: &str) -> Result<Value> {
    let rows = r.rows(family);
    if rows.len() != 1 {
        return Err("expected one actual launch row".into());
    }
    rows.into_iter().next().ok_or_else(|| "actual row".into())
}
fn paid(r: &Rig) -> Result<PersonalPaidLicence> {
    Ok(serde_json::from_value(one(
        r,
        "roll_mind_personal_paid_licence",
    )?)?)
}
fn term(r: &Rig) -> Result<PaidTermRecord> {
    let licence = paid(r)?;
    r.rows("roll_mind_paid_term")
        .into_iter()
        .map(serde_json::from_value::<PaidTermRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .find(|t| t.licence() == licence.id() && t.generation() == licence.term())
        .ok_or_else(|| "actual current paid term".into())
}
/// An owner licence's one owner term, as the launch's actual filing wrote it.
fn owner_term(r: &Rig, licence: LicenceId) -> Result<OwnerTerm> {
    let mut terms = r
        .rows("roll_mind_owner_term")
        .into_iter()
        .map(serde_json::from_value::<OwnerTerm>)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|t| t.licence() == licence);
    match (terms.next(), terms.next()) {
        (Some(term), None) => Ok(term),
        _ => Err("expected the owner licence's one term".into()),
    }
}
/// A window's (term, Grace) lengths in IC seconds.
fn lengths(window: TermWindow) -> (i64, i64) {
    (
        window.term_seconds(),
        window.grace_end().get() - window.term_end().get(),
    )
}
fn fence(r: &mut Rig, name: &str, parameters: Value) -> Result {
    let buyer = r.buyer;
    member(r, false)?;
    check!(crate::test_checks::refused(r.plan(
        "market",
        name,
        buyer,
        parameters.clone()
    ))?);
    member(r, true)?;
    let planned = r.plan("market", name, buyer, parameters.clone())?;
    member(r, false)?;
    check!(r.stage(planned).is_err(), "membership lost before stage");
    member(r, true)?;
    let planned = r.plan("market", name, buyer, parameters)?;
    r.stage(planned)?;
    member(r, false)?;
    check!(
        r.world_mut().engine_mut().take_staged_action().is_err(),
        "membership lost before consuming take"
    );
    member(r, true)?;
    Ok(())
}

/// 17g C1c's three resale checks on the settled House Pin `licence`, while its
/// buyer is a Republic member (C1 plan §4.4 test 5, §5):
/// - (a) the paid-resale projected actions are byte-identical for the
///   holder, God and the Registry operator, as JSON with their per-actor
///   `offer` ids removed: offered to every live player, revealing no holding.
///   Since 17g C2a each kind has its own verb, so the check reads the set of
///   all three (`paid-mind-resale-offer`, `-accept`, `-fund`), never Offer
///   alone;
/// - (b) the holder's paid-resale Offer naming the clerk prepares. It is
///   planned only, never staged, so no later step of the story moves;
/// - (c) God's identical Offer refuses, with the same bytes as God's Offer on
///   a licence that does not exist: the invocation reveals no holding either.
fn resale_privacy(r: &mut Rig, d: &LaunchDeployment, licence: u64) -> Result {
    let market = d.market.clone().ok_or("a format-2 market")?;
    let operator = operator(d)?;
    let buyer = r.buyer;
    let resale = |r: &mut Rig, actor: ObjId| -> Result<Vec<Value>> {
        let mut offers = Vec::new();
        for offer in projected(r, actor)? {
            if !matches!(
                offer.presentation().verb.as_str(),
                "paid-mind-resale-offer" | "paid-mind-resale-accept" | "paid-mind-resale-fund"
            ) {
                continue;
            }
            let mut value = serde_json::to_value(&offer)?;
            value
                .as_object_mut()
                .ok_or("a projected action is an object")?
                .remove("offer")
                .ok_or("a projected action names its offer")?;
            offers.push(value);
        }
        Ok(offers)
    };
    let held = resale(r, buyer)?;
    check!(
        !held.is_empty(),
        "the holder is offered the paid-resale verbs"
    );
    check_eq!(&held, &resale(r, GOD)?, "the holder and God");
    check_eq!(&held, &resale(r, operator)?, "the holder and the operator");
    eprintln!(
        "NAMED_LAUNCH_RESALE projected={} entries={}",
        held.len(),
        serde_json::to_string(&held)?
    );
    // The holder's own personal account takes the remainder.
    let seller_account = r.payer;
    let offer = |licence: u64| json!({"licence":licence,"buyer":market.clerk.0,"price":40000,"seller_account":seller_account});
    let (held_offer, absent_offer) = (offer(licence), offer(licence + 1_000_000));
    r.plan(
        "market",
        mush_roll::commerce::paid_resale::protocol::OFFER,
        buyer,
        held_offer.clone(),
    )?;
    let refusal = |result: Result<driver::Planned>| -> Result<String> {
        match result {
            Ok(_) => Err("God's paid-resale Offer prepared".into()),
            Err(error)
                if error
                    .downcast_ref::<crate::test_checks::CheckFailed>()
                    .is_some() =>
            {
                Err(error)
            }
            Err(error) => Ok(error.to_string()),
        }
    };
    let on_the_holding = refusal(r.plan(
        "market",
        mush_roll::commerce::paid_resale::protocol::OFFER,
        GOD,
        held_offer,
    ))?;
    let on_nothing = refusal(r.plan(
        "market",
        mush_roll::commerce::paid_resale::protocol::OFFER,
        GOD,
        absent_offer,
    ))?;
    eprintln!("NAMED_LAUNCH_RESALE refusal={on_the_holding:?}");
    check_eq!(
        on_the_holding,
        on_nothing,
        "God's refusal on the buyer's Pin equals a missing licence's"
    );
    Ok(())
}

async fn other_named_house(
    r: &mut Rig,
    d: &LaunchDeployment,
    recipe: LaunchRecipe,
    house: &HouseDeployment,
    selector: &str,
) -> Result {
    command(
        r.world_mut(),
        GOD,
        &format!("@mind/launch {selector}={}", house.source_name),
    )?;
    let ordinal = match recipe {
        LaunchRecipe::Coprates => 1,
        LaunchRecipe::Meridiani => 2,
        LaunchRecipe::SafeConduct => 3,
    };
    let (revision, rating) = certify(r, &house.source_name, ordinal).await?;
    assert_eq!(
        revision.source(),
        mush_roll::source_digest(&recipe.draft()?.source)
    );
    let (shell, _) = r.perform(
        "roll",
        mush_core::reserved_shell::RESERVE_ACTION,
        GOD,
        json!({}),
    )?;
    let treasury_before = r.money(house.treasury)?;
    let reference = revision.reference();
    let (filing, _) = r.perform(
        "roll",
        mush_economy::registry_filing::HOUSE_FILING_ACTION,
        GOD,
        json!({
            "shell_request":RequestUid::from_bytes(*shell.as_bytes()),
            "mind":reference.key.mind,"revision":reference.key.number,
            "revision_digest":reference.digest,"rating_job":rating.job(),
            "rating_id":rating.id(),"rating_digest":rating.receipt_digest(),
            "payer":house.treasury,"charter_name":house.source_name,
            "organization":house.organization,"scope":house.scope
        }),
    )?;
    assert_eq!(treasury_before.0 - r.money(house.treasury)?.0, 10000);
    let birth = r
        .rows("roll_mind_initial_filing")
        .into_iter()
        .find(|row| row.get("commit") == Some(&json!(filing)))
        .ok_or("actual named filing")?;
    let checked: mush_roll::commerce::records::InitialFilingOperation =
        serde_json::from_value(birth.clone())?;
    assert_eq!(
        checked.revision(),
        mush_roll::commerce::RevisionIdentity::capture(&revision)?
    );
    let owner_licence: LicenceId = serde_json::from_value(
        birth
            .get("licence")
            .ok_or("actual named owner licence")?
            .clone(),
    )?;
    let buyer = r.buyer;
    for actor in [GOD, buyer] {
        r.world_mut().world_mut().move_to(actor, d.venue.room)?;
    }
    r.flush();
    let (listing, _) = r.perform(
        "market",
        "list_house_mind",
        GOD,
        json!({"owner_licence":owner_licence}),
    )?;
    let listed = r
        .rows("market_mind_house_listing")
        .into_iter()
        .find(|row| row.get("publication") == Some(&json!(listing)))
        .ok_or("actual acknowledged named listing")?;
    let listed: mush_market::mind_house::HouseListingV1 = serde_json::from_value(listed)?;
    assert_eq!(listed.publication(), listing);
    listed.digest()?;
    let (quote, _) = r.perform(
        "market",
        "quote_house_mind",
        buyer,
        json!({"listing":listing}),
    )?;
    let before = r.money(r.payer)?;
    let (fund, _) = r.perform(
        "market",
        "purchase_house_mind",
        buyer,
        json!({"quote":quote}),
    )?;
    r.perform(
        "market",
        "settle_house_mind",
        buyer,
        json!({"operation":fund}),
    )?;
    assert_eq!(before.0 - r.money(r.payer)?.0, 40000);
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "full three-source Registry measurement; run the exact named launch acceptance gate"]
async fn named_coprates_real_save_rate_house_sale_and_membership_fences() -> Result {
    // 17g C1c: the production format (2), with the player market composed
    // after the House chain, and the runbook's Registry grants at Initialize.
    let (mut r, mut d) = setup()?;
    reboot(&mut r, &d)?;
    r.initialize_roll()?;
    for action in [
        "initialize_core_origin",
        "initialize_core_production",
        "initialize_core_refit",
    ] {
        r.perform("industry", action, GOD, json!({}))?;
    }
    grant_registry(&mut r, &d)?;
    d.mode = LaunchMode::Active;
    reboot(&mut r, &d)?;
    connected(&mut r)?;
    command(r.world_mut(), GOD, "@mind/launch coprates=Coprates Drill")?;
    let (revision, rating) = certify(&mut r, &d.houses.coprates.source_name, 1).await?;
    check_eq!(
        revision.source(),
        mush_roll::source_digest(&LaunchRecipe::Coprates.draft()?.source)
    );
    let (shell, _) = r.perform(
        "roll",
        mush_core::reserved_shell::RESERVE_ACTION,
        GOD,
        json!({}),
    )?;
    let reference = revision.reference();
    r.perform("roll",mush_economy::registry_filing::HOUSE_FILING_ACTION,GOD,json!({
        "shell_request":RequestUid::from_bytes(*shell.as_bytes()),"mind":reference.key.mind,"revision":reference.key.number,
        "revision_digest":reference.digest,"rating_job":rating.job(),"rating_id":rating.id(),"rating_digest":rating.receipt_digest(),
        "payer":d.houses.coprates.treasury,"charter_name":"Coprates Drill","organization":d.houses.coprates.organization,"scope":d.houses.coprates.scope
    }))?;
    let birth = one(&r, "roll_mind_initial_filing")?;
    let owner_licence: LicenceId =
        serde_json::from_value(birth.get("licence").ok_or("actual owner licence")?.clone())?;
    // 17g C0 (owner, 2026-10-25): the House owner's free licence runs 365 IC
    // days with the usual 3 of Grace, until House owner attestation exists.
    check_eq!(
        lengths(owner_term(&r, owner_licence)?.window()),
        (365 * 86_400, 3 * 86_400),
        "the House owner's term"
    );
    let buyer = r.buyer;
    for actor in [GOD, buyer] {
        r.world_mut().world_mut().move_to(actor, d.venue.room)?;
    }
    r.flush();
    let (listing, _) = r.perform(
        "market",
        "list_house_mind",
        GOD,
        json!({"owner_licence":owner_licence}),
    )?;
    check!(crate::test_checks::refused(r.plan(
        "market",
        "quote_house_mind",
        buyer,
        json!({"listing":listing})
    ))?);
    member(&mut r, true)?;
    let (quote, _) = r.perform(
        "market",
        "quote_house_mind",
        buyer,
        json!({"listing":listing}),
    )?;
    let before = r.money(r.payer)?;
    fence(&mut r, "purchase_house_mind", json!({"quote":quote}))?;
    check_eq!(r.money(r.payer)?, before);
    let (fund, _) = r.perform(
        "market",
        "purchase_house_mind",
        buyer,
        json!({"quote":quote}),
    )?;
    fence(&mut r, "settle_house_mind", json!({"operation":fund}))?;
    r.perform(
        "market",
        "settle_house_mind",
        buyer,
        json!({"operation":fund}),
    )?;
    let accepted = paid(&r)?;
    let accepted_term = term(&r)?;
    // The paid House Pin keeps the paid 90 days beside the 365-day owner term.
    check_eq!(
        lengths(accepted_term.window()),
        (90 * 86_400, 3 * 86_400),
        "the paid House Pin's term"
    );
    check_eq!(before.0 - r.money(r.payer)?.0, 40000);
    resale_privacy(&mut r, &d, accepted.id().get())?;
    member(&mut r, false)?;
    reboot(&mut r, &d)?;
    check_eq!(paid(&r)?, accepted);
    check_eq!(
        term(&r)?,
        accepted_term,
        "loss of membership cannot revoke an accepted term"
    );
    check!(crate::test_checks::refused(r.plan(
        "market",
        "quote_house_mind",
        buyer,
        json!({"listing":listing})
    ))?);
    // The issuer's 365-day owner term is still Active at the buyer's actual due
    // boundary, 90 days in (before 17g C0 it was in Grace there). The next paid
    // term separately requires current membership.
    r.world_mut()
        .world_mut()
        .set_now(accepted_term.window().term_end().get());
    r.flush();
    use mush_roll::commerce::house_renewal::protocol::Transition as HouseRenewal;
    // `fence` starts from a member and proves each loss; rejoin first.
    member(&mut r, true)?;
    fence(
        &mut r,
        HouseRenewal::Draft.id(),
        json!({"licence":accepted.id().get()}),
    )?;
    let (draft, _) = r.perform(
        "market",
        HouseRenewal::Draft.id(),
        buyer,
        json!({"licence":accepted.id().get()}),
    )?;
    fence(&mut r, HouseRenewal::Fund.id(), json!({"operation":draft}))?;
    r.perform(
        "market",
        HouseRenewal::Fund.id(),
        buyer,
        json!({"operation":draft}),
    )?;
    fence(&mut r, HouseRenewal::Grant.id(), json!({"operation":draft}))?;
    r.perform(
        "market",
        HouseRenewal::Grant.id(),
        buyer,
        json!({"operation":draft}),
    )?;
    check_eq!(paid(&r)?.term().get(), 2);
    check_eq!(before.0 - r.money(r.payer)?.0, 80000);
    member(&mut r, false)?;
    let renewed = term(&r)?;
    reboot(&mut r, &d)?;
    check_eq!(term(&r)?, renewed);
    connected(&mut r)?;
    // The other two named publishers use the same actual services while the
    // buyer is outside Republic. Imports alone never stand in for publication.
    other_named_house(
        &mut r,
        &d,
        LaunchRecipe::Meridiani,
        &d.houses.meridiani,
        "meridiani",
    )
    .await?;
    other_named_house(
        &mut r,
        &d,
        LaunchRecipe::SafeConduct,
        &d.houses.tycho,
        "safe_conduct",
    )
    .await?;
    check_eq!(r.rows("roll_mind_initial_filing").len(), 3);
    check_eq!(r.rows("market_mind_house_listing").len(), 3);
    let holdings = r.rows("roll_mind_personal_paid_licence");
    check_eq!(holdings.len(), 3);
    for row in &holdings {
        let _: PersonalPaidLicence = serde_json::from_value(row.clone())?;
    }
    check_eq!(before.0 - r.money(r.payer)?.0, 160000);
    reboot(&mut r, &d)?;
    check_eq!(r.rows("roll_mind_personal_paid_licence"), holdings);
    // 17g C0: the Coprates catalogue sells for the whole 365-day owner term and
    // its Grace, and refuses once the owner's Grace ends. A quote moves no money.
    let owner = owner_term(&r, owner_licence)?.window();
    member(&mut r, true)?;
    for (at, sells) in [
        (owner.term_end().get() - 1, true),
        (owner.term_end().get() + 86_400, true),
        (owner.grace_end().get(), false),
    ] {
        r.world_mut().world_mut().set_now(at);
        r.flush();
        let quote = json!({"listing":listing});
        if sells {
            r.perform("market", "quote_house_mind", buyer, quote)?;
        } else {
            check!(
                crate::test_checks::refused(r.plan("market", "quote_house_mind", buyer, quote))?,
                "the catalogue sells past the House owner's Grace"
            );
        }
    }
    Ok(())
}
