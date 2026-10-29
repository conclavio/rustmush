//! Doc 17h §3 M8h (the lead's rulings A1–A3): the `tacsim.house_attestation`
//! tick end to end in the House world, booted with the owner attestations
//! (the House arm among them, under the House's 365-day terms, as the launch
//! composes) and the original-holdings read the tick walks the owner licences
//! through. The House filing's owner licence is the rig's own; the agent is an
//! NPC character on a login of its own, granted `roll.holder.org` on the
//! House's scope with a limit (the attestation moves no money, so a limited
//! grant attests: M8f H7).
//!
//! The tick is fired by the engine's own `fire_due`, and the step it queues as
//! the agent is landed through the agent's projected offer of the House
//! action, as the server's driver lands it; a step the provider refuses is
//! recorded as refused. Every phase starts from the same House filing.
use super::*;
use crate::house_attestation::HouseAttestationState;

const TICK: u128 = 21_000_000;
const HOUSE_ATTEST: &str = "attest_house_mind";
const ATTEST_OPS: &str = "roll_mind_owner_attestation_operation";
const SEAL_MARK: &str = "record_licence_seal_mark";
/// Why the last landing was refused (the failure messages print it).
static REFUSED: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
fn refused(why: impl std::fmt::Display) -> TestResult<bool> {
    *REFUSED.lock().map_err(module_error)? = Some(why.to_string());
    Ok(false)
}

/// The House world with the owner attestations and the original-holdings read
/// composed.
fn boot_tick(registry: &Fixture, saved: &Durable) -> TestResult<PlayerWorld> {
    boot_every_option(registry, saved, false, false, None, true, true)
}

/// What a launch's finalize sets (the rig composes no launch): the House's
/// attesting character, and the offer keys the agent's offers are signed with.
fn arm(w: &mut PlayerWorld, org: OrgId, agent: ObjId) -> TestResult {
    w.engine
        .world
        .module_state_mut::<HouseAttestationState>()
        .ok_or("the House attestation slot")?
        .set_agents(vec![(org, agent)]);
    w.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x6c; 32])?,
        [],
    )?);
    Ok(())
}

fn describe(w: &PlayerWorld) -> String {
    let refusal = REFUSED
        .lock()
        .ok()
        .and_then(|r| r.clone())
        .unwrap_or_default();
    w.engine
        .world
        .module_state::<HouseAttestationState>()
        .map(HouseAttestationState::describe)
        .unwrap_or_default()
        + format!(" refused={refusal:?}").as_str()
}

/// A Roll action by id.
fn roll_action(w: &PlayerWorld, id: &str) -> TestResult<DefinitionRef> {
    w.engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|e| e.reference())
        .find(|r| {
            r.key().owner.as_str() == "roll"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
                && r.version() == 1
        })
        .cloned()
        .ok_or_else(|| format!("roll action {id}").into())
}

/// One direct preparation of a Roll action (no offer).
fn prepare_roll(
    w: &mut PlayerWorld,
    id: &str,
    actor: ObjId,
    n: u128,
    params: Value,
) -> TestResult<PreparedComposedTxn> {
    let a = roll_action(w, id)?;
    Ok(w.engine
        .prepare_registered_composed_authoritative_action(
            &a,
            &request(n, actor, "roll", params)?,
            token(n),
        )?
        .0)
}

/// Land one queued submission through its actor's own projected offer:
/// `true` when it committed, `false` when it was refused (no offer, or the
/// provider's refusal at preparation).
fn land(w: &mut PlayerWorld, submit: &mush_core::AuthoritativeSubmit, n: u128) -> TestResult<bool> {
    let actor = submit.actor;
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, submit.subject.clone())?;
    let offers = w.engine.project_legal_actions(&query)?;
    let Some(projected) = offers.projected_for_action(&submit.action) else {
        return refused("no projected offer");
    };
    let req = request(
        n,
        actor,
        submit.action.key().owner.as_str(),
        submit.parameters.clone(),
    )?;
    let resolved = match w.engine.resolve_legal_action(
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
            offer: projected.offer(),
            subject: submit.subject.clone(),
            targets: submit.targets.clone(),
            parameters: submit.parameters.clone(),
        },
    ) {
        Ok(resolved) => resolved,
        Err(error) => return refused(format!("resolve: {error}")),
    };
    let (action, request, _) = resolved.into_authoritative_parts();
    check_eq!(action, submit.action);
    let prepared = match w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(&action, &request, token(n))
    {
        Ok((prepared, _)) => prepared,
        Err(error) => return refused(format!("prepare: {error}")),
    };
    apply(w, prepared)?;
    Ok(true)
}

/// Six world seconds on and the engine's due ticks fired: the one step the
/// tick queued, if any (not landed).
fn fire(w: &mut PlayerWorld) -> TestResult<Option<Box<mush_core::AuthoritativeSubmit>>> {
    let now = w.engine.world.now().checked_add(6).ok_or("clock")?;
    w.engine.world.set_now(now);
    w.engine.fire_due(
        now.checked_mul(1000).ok_or("clock")?,
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    w.commit()?;
    Ok(w.engine.take_authoritative_submit())
}

/// One step the tick queued and whether it landed.
type Landed = Option<(String, bool)>;

/// `fire`, the step landed or refused.
fn pulse(w: &mut PlayerWorld, n: &mut u128) -> TestResult<Landed> {
    let Some(submit) = fire(w)? else {
        return Ok(None);
    };
    *n = n.checked_add(1).ok_or("serial")?;
    let id = submit.action.key().id.to_string();
    let landed = land(w, &submit, *n)?;
    Ok(Some((id, landed)))
}

/// `count` pulses that must queue nothing.
fn quiet(w: &mut PlayerWorld, n: &mut u128, count: usize, why: &str) -> TestResult {
    for _ in 0..count {
        check_eq!(pulse(w, n)?, None, "{why}: {}", describe(w));
    }
    Ok(())
}

/// The owner licence's current generation.
fn generation(w: &PlayerWorld, owner: LicenceId) -> TestResult<Value> {
    Ok(rows(&disk(w), "roll", "roll_mind_owner_licence")
        .into_iter()
        .find(|v| v.get("id") == Some(&json!(owner)))
        .and_then(|v| v.get("term").cloned())
        .ok_or("the House owner licence")?)
}

fn owner_term(
    w: &PlayerWorld,
    owner: LicenceId,
    generation: u64,
) -> TestResult<mush_roll::commerce::records::OwnerTerm> {
    Ok(serde_json::from_value(
        rows(&disk(w), "roll", "roll_mind_owner_term")
            .into_iter()
            .find(|v| {
                v.get("licence") == Some(&json!(owner))
                    && v.get("generation") == Some(&json!(generation.to_string()))
            })
            .ok_or("the House owner generation")?,
    )?)
}

/// The House attestations of `owner`, by the acting character.
fn attestations(w: &PlayerWorld, owner: LicenceId) -> Vec<Value> {
    rows(&disk(w), "roll", ATTEST_OPS)
        .into_iter()
        .filter(|v| v.get("licence") == Some(&json!(owner)))
        .filter_map(|v| v.get("owner").cloned())
        .collect()
}

/// The tick's House agent: an NPC character on a login of its own.
fn agent(registry: &Fixture, w: &mut PlayerWorld) -> TestResult<ObjId> {
    let agent =
        w.engine
            .world
            .create(ObjKind::Player, "House attestation agent", GOD, Some(ROOM))?;
    let mut values = registry.accounts.values.lock().map_err(module_error)?;
    registry.accounts.epoch.invalidate();
    values.insert(agent, mush_core::AccountId::mint("house-attestation", 0)?);
    Ok(agent)
}

fn grant(w: &mut PlayerWorld, who: ObjId, scope: OrgScopeId) -> TestResult {
    command(
        w,
        &format!(
            "@org/grant #{}=House {} scope=#{} limit=1",
            who.0,
            mush_roll::commerce::org_holder::HOLDER_ORG_CAPABILITY,
            scope.0
        ),
    )
}

/// The story (module note). Each phase reboots from the House filing, or from
/// the due snapshot phase 2 takes.
pub(super) fn exercise(
    registry: &Fixture,
    saved: &Durable,
    organization: OrgId,
    scope: OrgScopeId,
) -> TestResult {
    let birth = one(rows(&saved.disk, "roll", "roll_mind_initial_filing"))?;
    let owner: LicenceId =
        serde_json::from_value(birth.get("licence").cloned().ok_or("owner licence")?)?;
    let mut w = boot_tick(registry, saved)?;
    let house_agent = agent(registry, &mut w)?;
    let result = story(registry, &mut w, (organization, scope), owner, house_agent);
    let mut values = registry.accounts.values.lock().map_err(module_error)?;
    registry.accounts.epoch.invalidate();
    values.remove(&house_agent);
    result
}

#[allow(clippy::too_many_lines)] // one composed story, phase by phase
fn story(
    registry: &Fixture,
    w: &mut PlayerWorld,
    (organization, scope): (OrgId, OrgScopeId),
    owner: LicenceId,
    house_agent: ObjId,
) -> TestResult {
    arm(w, organization, house_agent)?;
    let mut n = TICK;
    let first = owner_term(w, owner, 1)?;

    // 1. Before the term ends nothing is due, and nothing is queued.
    quiet(w, &mut n, 4, "nothing is due before the term ends")?;
    check_eq!(generation(w, owner)?, json!("1"));

    // 2. Due, but the agent holds no grant: the tick pages and never attests.
    set_ic(w, first.window().term_end().get())?;
    quiet(w, &mut n, 4, "an agent without the grant attests nothing")?;
    check!(describe(w).contains("/grant"), "{}", describe(w));
    check!(attestations(w, owner).is_empty());
    check_eq!(generation(w, owner)?, json!("1"));
    // The grant, limited (the attestation moves no money: M8f H7).
    grant(w, house_agent, scope)?;
    let due = saved(w);

    // 3. The tick attests, as the agent, through the House action: the new
    //    generation is the House's 365-day term, the agent recorded as acting.
    let money = balance(
        w,
        wallet(w, EconomicOwner::Organization(organization), Some(scope))?,
    )?;
    check_eq!(
        pulse(w, &mut n)?,
        Some((HOUSE_ATTEST.to_owned(), true)),
        "{}",
        describe(w)
    );
    check_eq!(generation(w, owner)?, json!("2"));
    let second = owner_term(w, owner, 2)?;
    check_eq!(
        second.window().term_end().get() - second.window().issued_at().get(),
        HOUSE_OWNER_TERM_SECONDS,
        "A4: the House's 365-day term, renewed"
    );
    check_eq!(attestations(w, owner), vec![json!(house_agent)]);
    check_eq!(
        balance(
            w,
            wallet(w, EconomicOwner::Organization(organization), Some(scope))?
        )?,
        money,
        "the attestation is free"
    );
    quiet(w, &mut n, 4, "generation 2 is not due")?;
    check_eq!(attestations(w, owner).len(), 1);
    // A3: the provider refuses a second attestation of the generation, by the
    // agent and by a granted curator alike.
    grant(w, GOD, scope)?;
    let params = json!({"owner_licence": owner.get(), "org": organization.0});
    for who in [house_agent, GOD] {
        n += 1;
        check!(
            prepare_roll(w, HOUSE_ATTEST, who, n, params.clone()).is_err(),
            "a second attestation of the generation"
        );
    }

    // 4. A restart between due and attestation: the tick queues the
    //    attestation and the world restarts before it lands. The restarted
    //    tick reads the generation still due and attests it once.
    let mut cold = boot_tick(registry, &due)?;
    arm(&mut cold, organization, house_agent)?;
    let queued = fire(&mut cold)?.ok_or("the queued attestation")?;
    check_eq!(queued.action.key().id.as_str(), HOUSE_ATTEST);
    check_eq!(queued.actor, house_agent, "queued as the agent");
    let mut cold = boot_tick(registry, &saved(&cold))?;
    arm(&mut cold, organization, house_agent)?;
    check_eq!(generation(&cold, owner)?, json!("1"), "nothing landed");
    check_eq!(
        pulse(&mut cold, &mut n)?,
        Some((HOUSE_ATTEST.to_owned(), true)),
        "{}",
        describe(&cold)
    );
    quiet(&mut cold, &mut n, 4, "attested once")?;
    check_eq!(attestations(&cold, owner), vec![json!(house_agent)]);
    check_eq!(generation(&cold, owner)?, json!("2"));
    drop(cold);

    // 5. A human curator attests first: the tick does nothing for that
    //    generation.
    let mut curated = boot_tick(registry, &due)?;
    arm(&mut curated, organization, house_agent)?;
    grant(&mut curated, GOD, scope)?;
    n += 1;
    let prepared = prepare_roll(&mut curated, HOUSE_ATTEST, GOD, n, params.clone())?;
    apply(&mut curated, prepared)?;
    quiet(&mut curated, &mut n, 4, "the curator attested first")?;
    check_eq!(attestations(&curated, owner), vec![json!(GOD)]);
    check_eq!(generation(&curated, owner)?, json!("2"));
    drop(curated);

    // 6. An Unattestable filing: an ungoverned seal mark inside the term. The
    //    tick never attests it, logs it exactly once, and the generation
    //    lapses at the end of Grace.
    let mut marked = boot_tick(registry, &due)?;
    arm(&mut marked, organization, house_agent)?;
    let hull = marked
        .engine
        .world
        .create(ObjKind::Thing, "Hull Ungoverned", GOD, None)?;
    let at_ic = first
        .window()
        .issued_at()
        .get()
        .checked_add(10)
        .ok_or("time")?;
    n += 1;
    let prepared = prepare_roll(
        &mut marked,
        SEAL_MARK,
        GOD,
        n,
        json!({"licence": owner, "hull": hull, "at_ic": at_ic, "governed": false,
               "sortie": format!("{}@{at_ic}", hull.0)}),
    )?;
    apply(&mut marked, prepared)?;
    quiet(
        &mut marked,
        &mut n,
        6,
        "an Unattestable filing is never queued",
    )?;
    let unattestable = |w: &PlayerWorld| describe(w).matches("unattestable").count();
    check_eq!(unattestable(&marked), 1, "{}", describe(&marked));
    check!(attestations(&marked, owner).is_empty());
    set_ic(&mut marked, first.window().grace_end().get())?;
    quiet(&mut marked, &mut n, 4, "the lapsed generation is not due")?;
    check_eq!(unattestable(&marked), 1, "logged once");
    check_eq!(generation(&marked, owner)?, json!("1"));
    let now = mush_roll::commerce::WorldIcSeconds::from_stamp(&marked.engine.world.world_ic_now())?;
    check_eq!(
        first.window().phase_at(now),
        mush_roll::commerce::TermPhase::Lapsed,
        "the Unattestable generation lapses"
    );
    Ok(())
}
