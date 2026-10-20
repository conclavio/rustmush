//! The real named source is saved, rated and filed through its actual services.
//! This in-process driver installs the exact acknowledged owner union. Its
//! observation adapter models no transport absence and grants no authority.
use super::*;
use mush_core::{
    AuthoritativeSubmit, CanonicalId, IdempotencyKey, IdempotencyScope, Inbound,
    OfferExecutionContext, OfferSubmission, Outbox, PerceivedRef, RequestUid, ScopedIdempotencyKey,
};
use mush_roll::commerce::{
    LicenceId,
    paid::{PaidTermRecord, PersonalPaidLicence},
    rating::{RatingReceipt, RatingRef},
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
    assert_eq!(request.actor, GOD);
    assert!(request.targets.is_empty());
    assert_eq!(request.subject, PerceivedRef::SelfActor);
    assert_eq!(request.action.key().owner.as_str(), "roll");
    assert!(request.action.key().id.as_str().starts_with("rating_"));
    assert_eq!(
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
    assert_eq!(action, request.action);
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
    if let Some(when) = r.world().engine().sched.next_due() {
        r.world_mut().fire_due(when);
    }
    r.flush();
    if let Some(submitted) = r
        .world_mut()
        .engine_mut()
        .take_observed_authoritative_submit()
    {
        assert!(submitted.is_observed());
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
async fn certify(r: &mut Rig, name: &str) -> Result<(mush_roll::Revision, RatingRef)> {
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
                return Ok((revision, rating));
            }
        }
        for wire in r.rows("rating_jobs") {
            let job: mush_roll::commerce::rating::RatingJob = serde_json::from_value(wire)?;
            use mush_roll::commerce::rating::RatingJobState;
            if matches!(
                job.state(),
                RatingJobState::Failed { .. }
                    | RatingJobState::Cancelled {}
                    | RatingJobState::Interrupted {}
                    | RatingJobState::Nonconforming { .. }
            ) {
                return Err("actual named source did not certify".into());
            }
        }
        if Instant::now() >= deadline {
            return Err("actual named Registry service timed out".into());
        }
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
fn fence(r: &mut Rig, name: &str, parameters: Value) -> Result {
    let buyer = r.buyer;
    member(r, false)?;
    assert!(r.plan("market", name, buyer, parameters.clone()).is_err());
    member(r, true)?;
    let planned = r.plan("market", name, buyer, parameters.clone())?;
    member(r, false)?;
    assert!(r.stage(planned).is_err(), "membership lost before stage");
    member(r, true)?;
    let planned = r.plan("market", name, buyer, parameters)?;
    r.stage(planned)?;
    member(r, false)?;
    assert!(
        r.world_mut().engine_mut().take_staged_action().is_err(),
        "membership lost before consuming take"
    );
    member(r, true)?;
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
    let (revision, rating) = certify(r, &house.source_name).await?;
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
    d.mode = LaunchMode::Active;
    reboot(&mut r, &d)?;
    connected(&mut r)?;
    command(r.world_mut(), GOD, "@mind/launch coprates=Coprates Drill")?;
    let (revision, rating) = certify(&mut r, &d.houses.coprates.source_name).await?;
    assert_eq!(
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
    assert!(
        r.plan(
            "market",
            "quote_house_mind",
            buyer,
            json!({"listing":listing})
        )
        .is_err()
    );
    member(&mut r, true)?;
    let (quote, _) = r.perform(
        "market",
        "quote_house_mind",
        buyer,
        json!({"listing":listing}),
    )?;
    let before = r.money(r.payer)?;
    fence(&mut r, "purchase_house_mind", json!({"quote":quote}))?;
    assert_eq!(r.money(r.payer)?, before);
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
    assert_eq!(before.0 - r.money(r.payer)?.0, 40000);
    member(&mut r, false)?;
    reboot(&mut r, &d)?;
    assert_eq!(paid(&r)?, accepted);
    assert_eq!(
        term(&r)?,
        accepted_term,
        "loss of membership cannot revoke an accepted term"
    );
    assert!(
        r.plan(
            "market",
            "quote_house_mind",
            buyer,
            json!({"listing":listing})
        )
        .is_err()
    );
    // The issuer's original term is still in Grace at the buyer's actual due
    // boundary. The next paid term separately requires current membership.
    r.world_mut()
        .world_mut()
        .set_now(accepted_term.window().term_end().get());
    r.flush();
    use mush_economy::managed_house_renewal::{DRAFT_ACTION, FUND_ACTION, GRANT_ACTION};
    fence(&mut r, DRAFT_ACTION, json!({"licence":accepted.id().get()}))?;
    let (draft, _) = r.perform(
        "market",
        DRAFT_ACTION,
        buyer,
        json!({"licence":accepted.id().get()}),
    )?;
    fence(&mut r, FUND_ACTION, json!({"operation":draft}))?;
    r.perform("market", FUND_ACTION, buyer, json!({"operation":draft}))?;
    fence(&mut r, GRANT_ACTION, json!({"operation":draft}))?;
    r.perform("market", GRANT_ACTION, buyer, json!({"operation":draft}))?;
    assert_eq!(paid(&r)?.term().get(), 2);
    assert_eq!(before.0 - r.money(r.payer)?.0, 80000);
    member(&mut r, false)?;
    let renewed = term(&r)?;
    reboot(&mut r, &d)?;
    assert_eq!(term(&r)?, renewed);
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
    assert_eq!(r.rows("roll_mind_initial_filing").len(), 3);
    assert_eq!(r.rows("market_mind_house_listing").len(), 3);
    let holdings = r.rows("roll_mind_personal_paid_licence");
    assert_eq!(holdings.len(), 3);
    for row in &holdings {
        let _: PersonalPaidLicence = serde_json::from_value(row.clone())?;
    }
    assert_eq!(before.0 - r.money(r.payer)?.0, 160000);
    reboot(&mut r, &d)?;
    assert_eq!(r.rows("roll_mind_personal_paid_licence"), holdings);
    Ok(())
}
