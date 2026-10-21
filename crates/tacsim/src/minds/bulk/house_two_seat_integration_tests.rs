//! Actual versioned two-seat House purchase, four-owner warrants and paid resale.
//! The existing private fixture region event remains the only absent host boundary.
//! No licence, term, payment, certificate or held-seat row is fabricated.
use super::*;
use mush_roll::commerce::house_purchase::{HouseAllocationV1, HouseProtocol};
const FIRST: u128 = 66_000_000;
const TERMS: &str = "roll_mind_paid_term";
const BIRTH: &str = "roll_mind_house_purchase";
const ECONOMICS: &str = "roll_mind_house_term_economics";
const CLAIMS: &str = "roll_mind_ic_claim";

fn wallet(w: &IcWorld, owner: EconomicOwner) -> TestResult<WalletId> {
    let economy = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("Economy")?;
    let snapshot = economy.snapshot();
    let mut accounts = snapshot
        .accounts()
        .filter(|a| a.owner == owner && a.scope.is_none() && a.instrument.as_str() == "cr");
    let account = accounts.next().ok_or("actual personal account")?.id;
    assert!(accounts.next().is_none(), "ambiguous actual account");
    Ok(account)
}
fn act(
    w: &mut IcWorld,
    id: &str,
    version: u32,
    actor: ObjId,
    n: u128,
    params: Value,
) -> TestResult {
    let transition = match (id, version) {
        ("list_house_mind", 2) => Some("list"),
        ("quote_house_mind", 2) => Some("quote"),
        ("purchase_house_mind", 2) => Some("fund"),
        ("settle_house_mind", 2) => Some("grant"),
        ("refund_house_mind", 2) => Some("refund"),
        _ => None,
    };
    let Some(transition) = transition else {
        let prepared = prepare(w, id, version, actor, n, params, None)?;
        return land(w, prepared);
    };
    let action = exact(w, id, version)?;
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let projected = offers.projected();
    let verb = format!("house-{transition}-two-seat");
    let mut matching = projected
        .iter()
        .filter(|offer| offer.presentation().verb.eq_ignore_ascii_case(&verb));
    let offer = matching.next().ok_or_else(|| {
        // TEMP M7 diag
        let verbs: Vec<_> = projected.iter().map(|o| o.presentation().verb.to_string()).collect();
        let room = w.engine.world.containing_room(actor);
        let kind = w.engine.world.get(actor).map(|o| (o.kind, o.flags));
        format!(
            "actual HouseV2 public offer (wanted {verb} for {id} v{version}; actor {actor:?} kind {kind:?} room {room:?} base {:?}; projected {verbs:?})",
            w.base
        )
    })?;
    assert!(matching.next().is_none(), "ambiguous HouseV2 public offer");
    let request = request(n, actor, "market", params.clone())?;
    let resolved = w.engine.resolve_legal_action(
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
            parameters: params,
        },
    )?;
    let (actual, resolved, _) = resolved.into_authoritative_parts();
    assert_eq!(
        actual, action,
        "public selector resolved a different protocol"
    );
    let invocation = resolved.invocation().clone();
    let (p, output) = w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(&action, &resolved, token(n))?;
    let prepared = Box::new(Prepared {
        p,
        output,
        invocation,
        action,
        serial: n,
    });
    land(w, prepared)
}
fn paid_term(w: &IcWorld, generation: u64) -> TestResult<PaidTermRecord> {
    if std::env::var_os("M7_CONDUCT_DIAG").is_some() {
        // TEMP M7 diag
        let all = rows(&w.disk, "roll", TERMS);
        let gens: Vec<_> = all.iter().map(|r| (r["generation"].clone(), r["licence"].clone())).collect();
        eprintln!("M7_HOUSE_DIAG paid_term want={generation} rows={} (generation, licence)={gens:?}", all.len());
    }
    let value = one(rows(&w.disk, "roll", TERMS)
        .into_iter()
        // A term generation is stored as its canonical decimal string.
        .filter(|r| r["generation"] == json!(generation.to_string()))
        .collect())?;
    Ok(serde_json::from_value(value)?)
}
fn claim(w: &IcWorld, n: u128) -> TestResult<Value> {
    one(rows(&w.disk, "roll", CLAIMS)
        .into_iter()
        .filter(|r| r["open"]["operation"] == json!(token(n)))
        .collect())
}
fn close(w: &mut IcWorld, actor: ObjId, opening: u128, n: u128, hull: ObjId) -> TestResult {
    let event = ic_sortie::fixture_close(&w.engine.world, actor, token(opening), w.base)?;
    let ticket = w.host.enqueue_close(&event)?;
    w.land(&ticket, n, hull, true)
}

#[inline(never)]
fn open_purchased_house(
    registry: &Fixture,
    saved: &Durable,
) -> TestResult<(Box<IcWorld>, ObjId, WalletId, WalletId)> {
    let initial = one(rows(&saved.disk, "roll", "roll_mind_initial_filing"))?;
    let owner: LicenceId =
        serde_json::from_value(initial.get("licence").cloned().ok_or("House owner")?)?;
    let mut w = IcWorld::new_house(registry, saved)?;
    // A third real authored hull lets the story distinguish two seats from an
    // accidentally unbounded admission. It gets actual native unit and title rows.
    let third =
        w.engine
            .world
            .create(ObjKind::Thing, "IC two-seat third hull", GOD, Some(w.base))?;
    w.engine.world.attach_behavior(
        third,
        mush_core::BehaviorId::new("sim.participant")?,
        mush_core::BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: w.engine.world.now(),
        },
    )?;
    w.persist_setup()?;
    let authored = w.snapshot();
    let mut w = w.reboot_house(registry, &authored)?;
    w.command(&format!("@unit/create #{}=tacsim.unit.heavy_mk1", third.0))?;
    w.command(&format!("@asset/title #{}=#{}", third.0, GOD.0))?;
    w.command(&format!(
        "@spatial/place/interior #{}=#{}",
        third.0, w.base.0
    ))?;
    for actor in [GOD, BUYER] {
        w.command(&format!("@account/open #{}=cr", actor.0))?;
    }
    let seller_account = wallet(&w, EconomicOwner::Object(BUYER))?;
    let buyer_account = wallet(&w, EconomicOwner::Object(GOD))?;
    let faucet = wallet(
        &w,
        EconomicOwner::System(SystemPrincipalId::new("faucet").ok_or("faucet")?),
    )?;
    w.command(&format!("@econ/mint {faucet}={seller_account} 2000"))?;
    w.command(&format!("@econ/mint {faucet}={buyer_account} 2000"))?;
    // The House guard admits a requester only inside the venue: the bay is the
    // `fixture` venue here, as the one-seat House fixture seats both actors.
    for actor in [GOD, BUYER] {
        w.engine.world.move_to(actor, w.base)?;
    }
    w.persist_setup()?;
    for (id, actor, n, parameters) in [
        (
            "list_house_mind",
            GOD,
            FIRST,
            json!({"owner_licence":owner.get().to_string()}),
        ),
        (
            "quote_house_mind",
            BUYER,
            FIRST + 1,
            json!({"listing":token(FIRST)}),
        ),
        (
            "purchase_house_mind",
            BUYER,
            FIRST + 2,
            json!({"quote":token(FIRST+1)}),
        ),
        (
            "settle_house_mind",
            BUYER,
            FIRST + 3,
            json!({"operation":token(FIRST+2)}),
        ),
    ] {
        act(&mut w, id, 2, actor, n, parameters)?;
    }
    Ok((Box::new(w), third, seller_account, buyer_account))
}

#[inline(never)]
fn exercise_new_holder_renewal(
    registry: &Fixture,
    mut w: Box<IcWorld>,
    original: &PersonalPaidLicence,
    transferred: &PersonalPaidLicence,
    term: &mush_roll::commerce::paid::PaidTermRecord,
    birth: &[Value],
) -> TestResult {
    // The same acquired two-seat template survives a new-holder generation and
    // a real same-contract renewal; no ancestor licence is issued to this buyer.
    w.engine.world.set_now(term.window().term_end().get());
    act(
        &mut w,
        "draft_house_mind_renewal",
        1,
        GOD,
        FIRST + 30,
        json!({"licence":original.id().get()}),
    )?;
    assert_eq!(pointer(&w)?, *transferred, "draft changed paid use");
    act(
        &mut w,
        "fund_house_mind_renewal",
        1,
        GOD,
        FIRST + 31,
        json!({"operation":token(FIRST+30)}),
    )?;
    let mut w = w.reboot_house(registry, &w.snapshot())?;
    act(
        &mut w,
        "settle_house_mind_renewal",
        1,
        GOD,
        FIRST + 32,
        json!({"operation":token(FIRST+30)}),
    )?;
    let renewed = pointer(&w)?;
    assert_eq!(renewed.term().get(), 2);
    assert_eq!(renewed.ownership(), transferred.ownership());
    assert_eq!(renewed.operation(), original.operation());
    assert_eq!(paid_term(&w, 1)?, *term);
    assert_eq!(paid_term(&w, 2)?.terms().seats().get(), 2);
    assert_eq!(rows(&w.disk, "roll", BIRTH).as_slice(), birth);
    let cold = w.reboot_house(registry, &w.snapshot())?;
    assert_eq!(pointer(&cold)?, renewed);
    assert_eq!(paid_term(&cold, 2)?, paid_term(&w, 2)?);
    Ok(())
}

#[inline(never)]
pub(in super::super) fn exercise(registry: &Fixture, saved: &Durable) -> TestResult {
    let (mut w, third, seller_account, buyer_account) = open_purchased_house(registry, saved)?;
    if rows(&w.disk, "roll", BIRTH).len() != 1 {
        // TEMP M7 diag
        let mut families = std::collections::BTreeMap::<String, usize>::new();
        for ((ns, _), v) in w.disk.iter() {
            let family = v
                .get("family")
                .and_then(|f| f.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("-");
            *families.entry(format!("{ns}/{family}")).or_default() += 1;
        }
        eprintln!("M7_HOUSE_DIAG disk families after two-seat purchase: {families:?}");
    }
    let allocation: HouseAllocationV1 = serde_json::from_value(one(rows(&w.disk, "roll", BIRTH))?)?;
    assert_eq!(allocation.protocol(), HouseProtocol::HouseV2);
    let original = pointer(&w)?;
    let term = paid_term(&w, 1)?;
    assert_eq!(term.terms().seats().get(), 2);
    assert!(term.terms().transferable());
    assert!(matches!(
        term.terms().form(),
        mush_roll::commerce::paid::PaidForm::Sealed {}
    ));
    assert_eq!(original.id(), allocation.licence());
    let birth = rows(&w.disk, "roll", BIRTH);
    let economics = rows(&w.disk, "roll", ECONOMICS);
    let economics_row = economics.first().ok_or("actual House economics")?;
    assert_eq!(economics_row["format"], 2);
    assert_eq!(economics_row["grant"]["renewal"]["format"], 2);
    let first = w.hulls[0];
    let second = w.hulls[1];
    let opening = w.ticket(BUYER, first, original.id().get())?;
    w.land(&opening, FIRST + 10, first, false)?;
    let old_claim = claim(&w, FIRST + 10)?;
    assert_eq!(old_claim["format"], 1);
    // Real held warrant forces Fund, even though another purchased seat is free.
    offer(&mut w, FIRST + 11, BUYER, GOD, seller_account.0)?;
    assert!(
        prepare(
            &mut w,
            ACCEPT,
            1,
            GOD,
            FIRST + 12,
            json!({"offer":token(FIRST+11),"payer":buyer_account.0}),
            None
        )
        .is_err()
    );
    buy(&mut w, FIRST + 13, FIRST + 11, FUND, GOD, buyer_account.0)?;
    assert_eq!(pointer(&w)?, original);
    assert!(
        w.ticket(BUYER, second, original.id().get())
            .and_then(|t| w.prepare(&t, FIRST + 14))
            .is_err(),
        "pending seller obtained a new seal"
    );
    assert!(
        prepare(
            &mut w,
            "draft_house_mind_renewal",
            1,
            BUYER,
            FIRST + 15,
            json!({"licence":original.id().get()}),
            None
        )
        .is_err()
    );
    let funded = w.snapshot();
    let mut w = w.reboot_house(registry, &funded)?;
    assert!(w.held(first)?);
    let consent = history(&w, FIRST + 13)?.consent().clone();
    assert_eq!(consent.claims.open_paid, 1);
    assert_eq!(consent.terms.basis.licence, original.id().get());
    // WorldIC is the actual fixture default ratio1/offset0; no claim deadline
    // or captured term is shortened to cause this service settlement.
    w.engine.world.set_now(consent.deadline.at);
    w.engine.set_submission_runtime(Arc::new(Runtime))?;
    let (request, _) = next(&mut w, SETTLE)?;
    let prepared = terminal(&mut w, &request, FIRST + 16)?;
    land(&mut w, prepared)?;
    assert_eq!(
        history(&w, FIRST + 13)?.outcome(),
        mush_economy::managed_paid_resale::PaidResaleOutcome::Settled
    );
    let transferred = pointer(&w)?;
    assert_eq!(transferred.holder(), GOD);
    assert_eq!(transferred.term(), original.term());
    assert_eq!(transferred.operation(), original.operation());
    assert_eq!(paid_term(&w, 1)?, term);
    assert_eq!(rows(&w.disk, "roll", BIRTH), birth);
    assert_eq!(rows(&w.disk, "roll", ECONOMICS), economics);
    assert_eq!(
        claim(&w, FIRST + 10)?,
        old_claim,
        "deadline changed the seller's warrant"
    );
    assert!(w.held(first)?);
    w.command(&format!("@asset/transfer #{}=#{}", second.0, GOD.0))?;
    let buyer_open = w.ticket(GOD, second, original.id().get())?;
    w.land(&buyer_open, FIRST + 17, second, false)?;
    let captured = claim(&w, FIRST + 17)?;
    assert_eq!(captured["format"], 3);
    assert_eq!(captured["open"]["paid_ownership"]["generation"], 2);
    assert!(
        w.ticket(GOD, third, original.id().get())
            .and_then(|t| w.prepare(&t, FIRST + 18))
            .is_err(),
        "buyer exceeded two minus the seller's held seat"
    );
    let mut w = w.reboot_house(registry, &w.snapshot())?;
    assert!(w.held(first)? && w.held(second)?);
    assert!(
        w.ticket(GOD, third, original.id().get())
            .and_then(|t| w.prepare(&t, FIRST + 19))
            .is_err(),
        "cold boot lost held seats"
    );
    close(&mut w, BUYER, FIRST + 10, FIRST + 20, first)?;
    let freed = w.ticket(GOD, third, original.id().get())?;
    w.land(&freed, FIRST + 21, third, false)?;
    assert!(w.held(second)? && w.held(third)?);
    close(&mut w, GOD, FIRST + 17, FIRST + 22, second)?;
    close(&mut w, GOD, FIRST + 21, FIRST + 23, third)?;
    let mut w = w.reboot_house(registry, &w.snapshot())?;
    assert_eq!(pointer(&w)?, transferred);
    assert!(!w.held(first)? && !w.held(second)? && !w.held(third)?);
    exercise_new_holder_renewal(
        registry,
        Box::new(w),
        &original,
        &transferred,
        &term,
        &birth,
    )?;
    Ok(())
}
