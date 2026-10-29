//! Doc 17h §3 M8f (the lead's fix round 1, H8): `tacsim.procurement` end to
//! end in the composed entitlement world. The setup:
//! - a House organisation with a treasury and a policy;
//! - an NPC agent with a login account of its own (ruling A), standing at the
//!   venue under a `roll.holder.org` grant limited to the monthly budget
//!   (`@org/grant … limit=`, P2a);
//! - a human member of the House.
//!
//! The tick is fired by the engine's own `fire_due`, and every step it queues
//! as the agent is landed through the agent's projected offer (the House
//! renewal service's `land` precedent). A step the provider refuses is
//! recorded as refused, as the server's driver reports it. Nothing is
//! constructed: the quotes, the escrow, the grant and the ledger rows are the
//! providers' own.
//!
//! **Hulls.** The rig loads no programme port, so
//! `surface::recovery::world_hulls` is empty, and an economy title there can
//! never count as a hull. Seeding hull titles through the economy would not
//! change `H`. The cap's `H` is therefore taken as ten through the tick's
//! test-only seam (`ProcurementState::assume_hulls`); the cap's arithmetic is
//! covered purely (`procurement::tests`).
use super::*;
use crate::procurement::tick::ProcurementState;

const PROC: u128 = FIRST + 1_200;
/// The fixture listing's price, and the House's monthly budget: one purchase
/// spends it.
const BUDGET: i64 = 100;

fn policy(
    org: mush_org::OrgId,
    root: mush_org::OrgScopeId,
    agent: ObjId,
) -> crate::launch::ProcurementPolicy {
    crate::launch::ProcurementPolicy {
        org,
        scope: root,
        agent,
        budget_minor_per_ic_month: Some(BUDGET),
        standing: None,
        cap_permille: None,
        roles: crate::launch::ProcurementRole::all(),
    }
}

/// What a launch's finalize sets (the rig composes no launch), plus `H` and,
/// when `zero`, the 0‰ line admitted (the rig's one genuine rating won none of
/// its trials; H9 keeps such a line out, which step 3 below asserts first).
fn arm(
    world: &mut PlayerWorld,
    policy: &crate::launch::ProcurementPolicy,
    zero: bool,
) -> TestResult {
    let state = world
        .engine
        .world
        .module_state_mut::<ProcurementState>()
        .ok_or("the procurement slot")?;
    state.set_policies(vec![policy.clone()]);
    state.assume_hulls(10);
    if zero {
        state.admit_zero_scores();
    }
    world.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x6b; 32])?,
        [],
    )?);
    Ok(())
}

/// Land one queued submission through its actor's own projected offer:
/// `true` when it committed, `false` when it was refused (no offer, or the
/// provider's refusal at preparation), as the server's driver answers it.
fn land(
    world: &mut PlayerWorld,
    submit: &mush_core::AuthoritativeSubmit,
    n: u128,
) -> TestResult<bool> {
    let actor = submit.actor;
    let principal = PrincipalId::Object { id: actor };
    let query =
        world
            .engine
            .legal_action_query(principal.clone(), actor, submit.subject.clone())?;
    let offers = world.engine.project_legal_actions(&query)?;
    let Some(projected) = offers.projected_for_action(&submit.action) else {
        return Ok(false);
    };
    let req = invocation(
        n,
        actor,
        submit.action.key().owner.as_str(),
        submit.parameters.clone(),
    )?;
    let Ok(resolved) = world.engine.resolve_legal_action(
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
    ) else {
        return Ok(false);
    };
    let (action, request, _) = resolved.into_authoritative_parts();
    check_eq!(action, submit.action);
    let Ok((prepared, _)) = world
        .engine
        .prepare_registered_composed_resolved_authoritative_action(&action, &request, token(n))
    else {
        return Ok(false);
    };
    let (writes, install) = prepared.into_parts();
    apply(world, &writes, install)?;
    Ok(true)
}

/// One step the tick queued and whether it landed.
type Landed = Option<(String, bool)>;

/// Six world seconds on, the engine's due ticks fired, and the one step the
/// procurement tick queued (if any) landed or refused.
fn pulse(world: &mut PlayerWorld, n: &mut u128) -> TestResult<Landed> {
    let now = world.engine.world.now().checked_add(6).ok_or("clock")?;
    world.engine.world.set_now(now);
    world.engine.fire_due(
        now.checked_mul(1000).ok_or("clock")?,
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    world.commit()?;
    let Some(submit) = world.engine.take_authoritative_submit() else {
        return Ok(None);
    };
    *n = n.checked_add(1).ok_or("serial")?;
    let id = submit.action.key().id.to_string();
    let landed = land(world, &submit, *n)?;
    Ok(Some((id, landed)))
}

/// The next step the tick queues within `max` pulses (a fresh month first
/// scans and scores, which queues nothing).
fn next_step(world: &mut PlayerWorld, n: &mut u128, max: usize) -> TestResult<Landed> {
    for _ in 0..max {
        if let Some(step) = pulse(world, n)? {
            return Ok(Some(step));
        }
    }
    Ok(None)
}

/// `count` pulses that must queue nothing.
fn quiet(world: &mut PlayerWorld, n: &mut u128, count: usize, why: &str) -> TestResult {
    for _ in 0..count {
        check_eq!(pulse(world, n)?, None, "{why}");
    }
    Ok(())
}

fn ledger(world: &PlayerWorld) -> Vec<Value> {
    rows(
        &disk(world),
        "tacsim",
        crate::procurement::PROCUREMENT_FAMILY,
    )
}

fn posted(world: &PlayerWorld, id: WalletId) -> TestResult<Amount> {
    Ok(store(world)?
        .snapshot()
        .account(id)
        .ok_or("account")?
        .posted_balance)
}

/// A restart: the captured store booted again, the census seated at the
/// venue (a boot seats it in the lobby) and the launch's policy re-armed.
fn restart(
    f: &PurchaseFixture,
    world: &mut PlayerWorld,
    house: &crate::launch::ProcurementPolicy,
) -> TestResult {
    restart_with(f, world, house, true)
}

/// `restart`, the 0‰ line admitted only when `zero`.
fn restart_with(
    f: &PurchaseFixture,
    world: &mut PlayerWorld,
    house: &crate::launch::ProcurementPolicy,
    zero: bool,
) -> TestResult {
    let snapshot = capture(world, &disk(world));
    *world = PlayerWorld::boot_entitlements(f, &snapshot)?;
    seat(world)?;
    arm(world, house, zero)
}

/// The fixture's Roll read as `actor` (`@procurement-read-fixture`).
fn roll_read(world: &mut PlayerWorld, actor: ObjId, org: mush_org::OrgId) -> TestResult<String> {
    let events = world.say(actor, &format!("@procurement-read-fixture {}", org.0))?;
    Ok(PlayerWorld::private_text(&events, actor))
}

pub(super) fn procurement_tick(
    f: &PurchaseFixture,
    granted: &Durable,
    owner: LicenceId,
) -> TestResult {
    let mut world = PlayerWorld::boot_entitlements(f, granted)?;
    cash(&mut world)?;
    world.say(GOD, "@org/create House")?;
    let (org, root) = {
        let store = world
            .engine
            .world
            .module_state::<mush_org::OrgStore>()
            .ok_or("Org")?
            .state();
        let house = store
            .orgs()
            .find(|o| o.name.as_str() == "House")
            .ok_or("the House")?;
        (house.id, house.root_scope)
    };
    let instrument = store(&world)?
        .snapshot()
        .account(WalletId(0))
        .ok_or("buyer wallet")?
        .instrument
        .clone();
    world.say(
        GOD,
        &format!("@account/open org:House/{}={}", root.0, instrument.as_str()),
    )?;
    let treasury = store(&world)?
        .snapshot()
        .accounts()
        .find(|a| a.owner == EconomicOwner::Organization(org) && a.scope == Some(root))
        .ok_or("the House's treasury")?
        .id;
    // The House's agent (an NPC character on a login account of its own, the
    // lead's ruling A) and a human member, both at the venue.
    let mut people = Vec::new();
    for (name, login) in [
        ("House Agent", "house-agent"),
        ("House Member", "house-member"),
    ] {
        let who = world
            .engine
            .world
            .create(mush_core::ObjKind::Player, name, GOD, Some(ROOM))?;
        let mut values = f.accounts.values.lock().map_err(module_error)?;
        f.accounts.epoch.invalidate();
        values.insert(who, mush_core::AccountId::mint(login, 0)?);
        people.push(who);
    }
    let [agent, member]: [ObjId; 2] = people.clone().try_into().map_err(|_| "two people")?;
    let result = run(f, &mut world, (org, root, treasury), (agent, member), owner);
    {
        let mut values = f.accounts.values.lock().map_err(module_error)?;
        f.accounts.epoch.invalidate();
        for who in people {
            values.remove(&who);
        }
    }
    result
}

#[allow(clippy::too_many_lines)] // one composed story, phase by phase
fn run(
    f: &PurchaseFixture,
    world: &mut PlayerWorld,
    (org, root, treasury): (mush_org::OrgId, mush_org::OrgScopeId, WalletId),
    (agent, member): (ObjId, ObjId),
    owner: LicenceId,
) -> TestResult {
    let fixture =
        EconomicOwner::System(SystemPrincipalId::new("purchase.fixture").ok_or("funding system")?);
    let faucet = store(world)?
        .snapshot()
        .accounts()
        .find(|a| a.owner == fixture)
        .ok_or("actual existing faucet")?
        .id;
    // P2a: `@org/grant … roll.holder.org limit=` is admitted; the agent's grant
    // is limited to the monthly budget, the member's is unlimited.
    let granted_agent = world.say(
        GOD,
        &format!(
            "@org/grant #{}=House {} limit={BUDGET}",
            agent.0,
            mush_roll::commerce::org_holder::HOLDER_ORG_CAPABILITY
        ),
    )?;
    let granted_agent = PlayerWorld::private_text(&granted_agent, GOD);
    world.say(
        GOD,
        &format!(
            "@org/grant #{}=House {}",
            member.0,
            mush_roll::commerce::org_holder::HOLDER_ORG_CAPABILITY
        ),
    )?;
    check!(
        mush_roll::commerce::org_holder::reads_for(&world.engine.world, org, agent).is_ok(),
        "the limited grant reads ({:?}; @org/grant answered {granted_agent:?})",
        mush_roll::commerce::org_holder::reads_for(&world.engine.world, org, agent)
    );
    check!(
        mush_roll::commerce::org_holder::acts_for(&world.engine.world, org, agent).is_err(),
        "an amountless act still fails closed for a limited grant"
    );
    let house = policy(org, root, agent);
    arm(world, &house, true)?;
    let mut n = PROC;

    // 1. The rig's child line is listed (its earlier phases listed it once; it
    //    is listed again here), while the parent's line, filed and held, is
    //    NOT listed. The treasury is empty: the month closes (the ore-bid
    //    precedent), and nothing is bought. The restart reopens the month
    //    from the rows alone.
    quiet(world, &mut n, 6, "an empty treasury buys nothing")?;
    check!(ledger(world).is_empty());
    list(world, owner, n + 1)?;
    n += 1;
    restart(f, world, &house)?;
    quiet(
        world,
        &mut n,
        6,
        "a treasury that cannot pay closes the month",
    )?;
    check!(ledger(world).is_empty());

    // 3. H9: funded now, but the line scored 0‰ (the rig's one genuine rating
    //    won none of its 12 trials): it is no candidate, and nothing is bought.
    world.say(GOD, &format!("@econ/mint {faucet}={treasury} 1000"))?;
    restart_with(f, world, &house, false)?;
    quiet(world, &mut n, 6, "a 0 per mille line is never a candidate")?;
    check!(ledger(world).is_empty());
    // H2 (the 0‰ line admitted from here on, through the test seam): only the
    //    FUND is refused. The agent quotes a listing; GOD then holds 950 of the
    //    treasury's 1000 (a Quote freezes no money, so the hold cannot refuse
    //    it, and the tick's treasury check ran before it), and the Fund fails
    //    on funds. With the hold released, the listing is skipped for the
    //    month: the agent quotes the line's OTHER listing, never the refused
    //    one again; held again, that Fund is refused too, both pages are
    //    paged, and with every listing skipped the month closes.
    restart(f, world, &house)?;
    let describe = |world: &PlayerWorld| {
        world
            .engine
            .world
            .module_state::<ProcurementState>()
            .map(|s| s.describe(org))
            .unwrap_or_default()
    };
    let hold = |world: &mut PlayerWorld| -> TestResult {
        world.say(GOD, &format!("@hold/place {treasury}=950"))?;
        check_eq!(
            store(world)?
                .snapshot()
                .account(treasury)
                .ok_or("treasury")?
                .held_amount,
            Amount::from_minor(950),
            "GOD's hold is placed"
        );
        Ok(())
    };
    let release = |world: &mut PlayerWorld| -> TestResult {
        let id = store(world)?
            .snapshot()
            .holds_of(treasury)
            .find(|h| h.status == mush_economy::HoldStatus::Held)
            .ok_or("GOD's hold")?
            .id;
        world.say(GOD, &format!("@hold/release {}", id.0))?;
        check_eq!(
            store(world)?
                .snapshot()
                .account(treasury)
                .ok_or("treasury")?
                .held_amount,
            Amount::ZERO,
            "GOD's hold is released"
        );
        Ok(())
    };
    // The listing the agent's newest quote names.
    let quoted = |world: &PlayerWorld| {
        rows(&disk(world), "market", "market_mind_entitlement_quote")
            .into_iter()
            .filter(|q| q.get("buyer") == Some(&json!({"org": org, "acting": agent})))
            .max_by_key(|q| {
                q.get("quoted_at_utc")
                    .and_then(Value::as_i64)
                    .unwrap_or_default()
            })
            .and_then(|q| q.get("listing").cloned())
            .ok_or("the agent's quote")
    };
    let mut refused_listings = Vec::new();
    for round in 0..2 {
        let step = next_step(world, &mut n, 4)?;
        check_eq!(
            step,
            Some(("quote_derived_mind".to_owned(), true)),
            "round {round}: {}",
            describe(world)
        );
        let listing = quoted(world)?;
        check!(
            !refused_listings.contains(&listing),
            "round {round}: a refused listing is never quoted again ({listing})"
        );
        hold(world)?;
        check_eq!(
            pulse(world, &mut n)?,
            Some(("purchase_derived_mind".to_owned(), false)),
            "round {round}: the Fund, and only the Fund, is refused"
        );
        // The next fire sees no funded purchase: the listing is skipped, the
        // page is paged, and nothing is queued.
        check_eq!(pulse(world, &mut n)?, None, "round {round}: no second Fund");
        let paged = describe(world);
        let text: mush_core::CommitToken = serde_json::from_value(listing.clone())?;
        check!(
            paged.contains(&format!("fund refused {text}")),
            "round {round}: {paged}"
        );
        release(world)?;
        refused_listings.push(listing);
    }
    check!(
        refused_listings.first() != refused_listings.last(),
        "the line's two listings, each quoted once"
    );
    quiet(
        world,
        &mut n,
        4,
        "every listing is skipped; the month is closed",
    )?;
    check!(
        describe(world).contains("nothing affordable under the cap is listed"),
        "{}",
        describe(world)
    );
    check!(ledger(world).is_empty());
    check_eq!(posted(world, treasury)?, Amount::from_minor(1000));
    check_eq!(
        store(world)?
            .snapshot()
            .account(treasury)
            .ok_or("treasury")?
            .held_amount,
        Amount::ZERO
    );

    // 4. The buy. H1: after the agent quotes, the human member quotes the same
    //    line for the House (a NEWER quote on the House's row); the agent
    //    funds its OWN quote, never the member's.
    restart(f, world, &house)?;
    let step = next_step(world, &mut n, 4)?;
    check_eq!(
        step,
        Some(("quote_derived_mind".to_owned(), true)),
        "{}",
        world
            .engine
            .world
            .module_state::<ProcurementState>()
            .map(|s| s.describe(org))
            .unwrap_or_default()
            + format!(
                " treasury={:?}",
                store(world)?.snapshot().account(treasury).map(|a| (
                    a.instrument.as_str().to_owned(),
                    a.posted_balance,
                    a.scope
                ))
            )
            .as_str()
    );
    // The listing the agent quoted: its newest quote for the House.
    let listing = quoted(world)?;
    commit(
        world,
        "market",
        "quote_derived_mind",
        1,
        member,
        n + 1,
        json!({"listing": listing, "org": org.0}),
    )?;
    n += 1;
    let member_quote = token(n);
    let money_before = balances(world)?;
    let mut steps = Vec::new();
    for _ in 0..24 {
        if let Some(step) = pulse(world, &mut n)? {
            if step.0 == "purchase_derived_mind" {
                let hold = store(world)?
                    .snapshot()
                    .holds_of(treasury)
                    .find(|h| h.status == mush_economy::HoldStatus::Held)
                    .ok_or("the treasury's hold")?
                    .clone();
                check_eq!(
                    hold.placed_by,
                    EconomicOwner::Organization(org),
                    "the hold is placed by the House"
                );
            }
            steps.push(step);
        }
        if ledger(world)
            .iter()
            .any(|row| row.get("phase") == Some(&json!("granted")))
        {
            break;
        }
    }
    let landed = |id: &str| (id.to_owned(), true);
    check_eq!(
        steps,
        vec![
            landed("purchase_derived_mind"),
            landed(crate::procurement::PROCUREMENT_ACTION),
            landed("settle_derived_mind"),
            landed(crate::procurement::PROCUREMENT_ACTION),
        ],
        "Fund, the ledger row (before the Grant), the Grant, the row's phase: one step a fire"
    );
    let [row] = ledger(world).try_into().map_err(|_| "one ledger row")?;
    check_eq!(row.get("listing"), Some(&listing));
    check_eq!(row.get("price_minor"), Some(&json!(BUDGET)));
    check_eq!(row.get("ordinal"), Some(&json!(0)));
    check_eq!(row.get("agent"), Some(&json!(agent)));
    let operation = row.get("operation").ok_or("the row's operation")?.clone();
    let allocation: EntitlementAllocationV1 = serde_json::from_value(
        rows(&disk(world), "roll", "roll_mind_entitlement_purchase")
            .into_iter()
            .find(|r| r.get("operation") == Some(&operation))
            .ok_or("the House's allocation")?,
    )?;
    check_eq!(
        allocation.holder(),
        mush_economy::Holder::Organization { org, acting: agent },
        "the licence is the House's, its agent acting"
    );
    check!(matches!(allocation.phase(), PurchasePhase::Granted { .. }));
    // P5: the House holds exactly the one line it bought; the parent's line,
    // filed and never listed, is untouched.
    check_eq!(
        rows(&disk(world), "roll", "roll_mind_entitlement_purchase")
            .into_iter()
            .filter_map(|r| serde_json::from_value::<EntitlementAllocationV1>(r).ok())
            .filter(|a| matches!(a.holder(), mush_economy::Holder::Organization { org: o, .. } if o == org))
            .count(),
        1,
        "the House bought only the listed line"
    );
    check!(
        allocation.quote() != member_quote,
        "H1: the agent funded its own quote, not the member's newer one"
    );
    // The split is balanced with the Registry levy (10) and the seller (90).
    let after = balances(world)?;
    for (id, posted_before, held) in &money_before {
        let delta = match id.0 {
            1 => 10,
            2 => 90,
            _ if *id == treasury => -BUDGET,
            _ => 0,
        };
        let actual = after
            .iter()
            .find(|(key, _, _)| key == id)
            .ok_or("retained account")?;
        check_eq!(
            actual.1,
            posted_before
                .checked_add(Amount::from_minor(delta))
                .ok_or("exact payout")?,
            "account {}",
            id.0
        );
        check_eq!(actual.2, *held);
    }

    // 5. A second fire in the same month buys nothing: the budget is spent.
    let rows_before = ledger(world);
    quiet(world, &mut n, 6, "the month's budget is spent")?;
    check_eq!(ledger(world), rows_before);

    // 6. The ledger's provider: a vacant key took a Funded row and that row
    //    changed only its phase (step 4); anything else is a Conflict.
    let granted: crate::procurement::ProcurementRow = serde_json::from_value(row.clone())?;
    for (forged, why) in [
        (
            granted.clone(),
            "a Granted row re-recorded: no Funded prior",
        ),
        (
            granted.with_phase(crate::procurement::RowPhase::Funded),
            "a Funded row at an occupied key",
        ),
    ] {
        let operation = forged.operation.to_string();
        world
            .engine
            .world
            .module_state_mut::<ProcurementState>()
            .ok_or("the procurement slot")?
            .offer_row(agent, forged);
        n += 1;
        let submit = mush_core::AuthoritativeSubmit {
            actor: agent,
            action: action(world, "tacsim", crate::procurement::PROCUREMENT_ACTION, 1)?,
            subject: PerceivedRef::SelfActor,
            targets: Vec::new(),
            parameters: json!({ "operation": operation }),
            key: mush_core::IdempotencyKeyText::parse_canonical(format!("procurement.forged.{n}"))?,
        };
        check!(!land(world, &submit, n)?, "{why}");
    }
    check_eq!(ledger(world), rows_before);

    // 7. A restart between fires changes nothing.
    let money = balances(world)?;
    restart(f, world, &house)?;
    quiet(world, &mut n, 6, "the rows are the memory")?;
    check_eq!(ledger(world), rows_before);
    check_eq!(balances(world)?, money);

    // 8. The Roll's procurement read is GOD's alone, through the composed
    //    reader; a lapsed licence holds no seat.
    check_eq!(roll_read(world, BUYER, org)?, "refused");
    check_eq!(roll_read(world, GOD, org)?, "holdings 1");
    let term = own_term(world, allocation.licence())?;
    let now = world.engine.world.now();
    world.engine.world.set_now(
        term.window()
            .grace_end()
            .get()
            .checked_add(1)
            .ok_or("time")?,
    );
    check_eq!(
        roll_read(world, GOD, org)?,
        "holdings 0",
        "a lapsed licence"
    );
    world.engine.world.set_now(now);

    // 9. H3: next month, the agent buys, then loses its grant between the
    //    Fund and the Grant, and the policy gains a backing faction the agent
    //    is no member of. Settling runs before the standing gate (and before
    //    the open-purchases read the revoked grant no longer passes): three
    //    Grants and three Refunds are still queued (and refused), the escrow is
    //    paged for staff, and the policy moves on: nothing more is queued.
    let month = crate::procurement::Period::of_ic(world.engine.world.world_ic_now().at);
    for _ in 0..64 {
        let next = world.engine.world.now().checked_add(86_400).ok_or("time")?;
        world.engine.world.set_now(next);
        if crate::procurement::Period::of_ic(world.engine.world.world_ic_now().at) != month {
            break;
        }
    }
    check!(
        crate::procurement::Period::of_ic(world.engine.world.world_ic_now().at) != month,
        "a new IC month"
    );
    restart(f, world, &house)?;
    let step = next_step(world, &mut n, 4)?;
    check_eq!(
        step,
        Some(("quote_derived_mind".to_owned(), true)),
        "{}",
        world
            .engine
            .world
            .module_state::<ProcurementState>()
            .map(|s| s.describe(org))
            .unwrap_or_default()
            + format!(
                " treasury={:?}",
                store(world)?.snapshot().account(treasury).map(|a| (
                    a.instrument.as_str().to_owned(),
                    a.posted_balance,
                    a.scope
                ))
            )
            .as_str()
    );
    check_eq!(
        pulse(world, &mut n)?,
        Some(("purchase_derived_mind".to_owned(), true))
    );
    check_eq!(
        pulse(world, &mut n)?,
        Some((crate::procurement::PROCUREMENT_ACTION.to_owned(), true)),
        "the Funded row, before any Grant"
    );
    world.say(GOD, &format!("@holder-grant-fixture revoke #{}", agent.0))?;
    let standing = crate::launch::ProcurementPolicy {
        standing: Some(mush_faction::FactionId(9_999)),
        ..house.clone()
    };
    arm(world, &standing, true)?;
    let mut refused = Vec::new();
    for _ in 0..12 {
        match pulse(world, &mut n)? {
            Some((id, false)) => refused.push(id),
            Some(step) => return Err(format!("{step:?} landed for a revoked agent").into()),
            None => break,
        }
    }
    check_eq!(
        refused,
        [
            "settle_derived_mind",
            "settle_derived_mind",
            "settle_derived_mind",
            "refund_derived_mind",
            "refund_derived_mind",
            "refund_derived_mind",
        ]
        .map(str::to_owned)
        .to_vec(),
        "the Grants, then the Refunds, each capped"
    );
    quiet(
        world,
        &mut n,
        4,
        "the escrow is paged and the policy moves on",
    )?;
    let paged = describe(world);
    check!(paged.contains("escrow held"), "{paged}");
    // No new buy follows: the revoked agent cannot read the House's open
    // purchases, so the visit is refused there (round 2), before the standing
    // gate is even asked.
    check!(paged.contains("/open"), "the new buy is refused: {paged}");
    Ok(())
}
