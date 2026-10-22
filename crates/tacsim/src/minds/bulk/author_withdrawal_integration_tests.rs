//! Real personal-author cause over the parent's genuine paid derivative.
//! Installation below models owner/Core ACK, not server transport.
use super::*;
use mush_core::{OfferExecutionContext, OfferSubmission};
const START: u128 = 13_000_000;
const ACTION: &str = "withdraw_authored_mind";
const CAUSES: &str = "roll_mind_author_withdrawal";
const STATUS: &str = "roll_mind_status";
fn params(a: &EntitlementAllocationV1) -> Value {
    json!({"filed":a.filed().get().to_string()})
}
fn prepare_withdraw(
    world: &mut PlayerWorld,
    actor: ObjId,
    number: u128,
    a: &EntitlementAllocationV1,
) -> TestResult<(
    PreparedComposedTxn,
    mush_core::action::PreparedAuthoritativeAction,
)> {
    prepare(world, "roll", ACTION, 1, actor, number, params(a))
}
fn signed(
    world: &mut PlayerWorld,
    number: u128,
    a: &EntitlementAllocationV1,
) -> TestResult<PreparedComposedTxn> {
    world.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x47; 32])?,
        [],
    )?);
    let actor = BUYER;
    let principal = PrincipalId::Object { id: actor };
    let query =
        world
            .engine
            .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = world.engine.project_legal_actions(&query)?;
    let action = action(world, "roll", ACTION, 1)?;
    let projected = offers
        .projected_for_action(&action)
        .ok_or("actual signed withdrawal offer")?;
    let inv = invocation(number, actor, "roll", params(a))?;
    let resolved = world.engine.resolve_legal_action(
        &offers,
        OfferExecutionContext {
            request: inv.request(),
            principal,
            actor,
            idempotency: inv.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        OfferSubmission {
            offer: projected.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: params(a),
        },
    )?;
    let (exact, resolved, _) = resolved.into_authoritative_parts();
    assert_eq!(exact, action);
    Ok(world
        .engine
        .prepare_registered_composed_resolved_authoritative_action(
            &action,
            &resolved,
            token(number),
        )?
        .0)
}
fn install(world: &mut PlayerWorld, prepared: PreparedComposedTxn) -> TestResult {
    let (writes, installation) = prepared.into_parts();
    apply(world, &writes, installation)
}
fn status(world: &PlayerWorld, a: &EntitlementAllocationV1) -> TestResult<Value> {
    rows(&disk(world), "roll", STATUS)
        .into_iter()
        .find(|v| v.get("filed") == Some(&json!(a.filed())))
        .ok_or_else(|| "actual filed status".into())
}
fn unchanged(before: &Disk, after: &Disk, a: &EntitlementAllocationV1) -> TestResult {
    for (key, value) in before {
        if value.pointer("/family/id") == Some(&json!(STATUS))
            && value.pointer("/value/filed") == Some(&json!(a.filed()))
        {
            continue;
        }
        assert!(
            after.get(key) == Some(value),
            "withdrawal changed preexisting non-status evidence"
        );
    }
    Ok(())
}
fn stage_withdraw(
    world: &mut PlayerWorld,
    number: u128,
    a: &EntitlementAllocationV1,
    prepared: PreparedComposedTxn,
    output: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let action = action(world, "roll", ACTION, 1)?;
    let inv = invocation(number, BUYER, "roll", params(a))?;
    let (outcome, resolution, mut definitions, eligible_witnesses, durable_outbox, semantic_events) =
        output.into_parts();
    definitions.push(action.clone());
    definitions.sort();
    definitions.dedup();
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(number),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([81; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([82; 32]),
        action,
        provider: OwnerId::new("roll")?,
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
fn transfer(world: &mut PlayerWorld, a: &EntitlementAllocationV1) -> TestResult {
    let charter = a.charter().object();
    world.transfer_charter(charter, GOD, 49_000_000)?;
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
        EconomicOwner::Object(GOD)
    );
    Ok(())
}
/// A fresh boot seats both actors in the lobby; the market venue is ROOM.
fn seat(world: &mut PlayerWorld) -> TestResult {
    for actor in [GOD, BUYER] {
        world.engine.world.move_to(actor, ROOM)?;
    }
    world.commit()
}
fn fences(f: &PurchaseFixture, granted: &Durable, a: &EntitlementAllocationV1) -> TestResult {
    let mut disabled = PlayerWorld::boot_entitlements(f, granted)?;
    assert!(action(&disabled, "roll", ACTION, 1).is_err());
    assert!(prepare_withdraw(&mut disabled, BUYER, START, a).is_err());
    for after_stage in [false, true] {
        let mut world = PlayerWorld::boot_withdrawal(f, granted)?;
        assert!(
            prepare_withdraw(&mut world, GOD, START, a).is_err(),
            "staff is not the personal author"
        );
        let (prepared, output) = prepare_withdraw(&mut world, BUYER, START + 1, a)?;
        // Core admits one pending commit: the title cannot move under a
        // prepared (or staged) withdrawal, so the transfer is refused and no
        // withdrawal cause is written.
        if after_stage {
            stage_withdraw(&mut world, START + 1, a, prepared, output)?;
            assert!(
                transfer(&mut world, a).is_err(),
                "title moved under a staged withdrawal"
            );
        } else {
            assert!(
                transfer(&mut world, a).is_err(),
                "title moved under a prepared withdrawal"
            );
            drop((prepared, output));
        }
        assert!(rows(&disk(&world), "roll", CAUSES).is_empty());
        assert_eq!(status(&world, a)?.get("withdrawn_by"), Some(&Value::Null));
        // After a real transfer on a clean engine the original author no
        // longer holds the title.
        let untouched = capture(&world, &disk(&world));
        let mut fresh = PlayerWorld::boot_withdrawal(f, &untouched)?;
        transfer(&mut fresh, a)?;
        assert!(
            prepare_withdraw(&mut fresh, BUYER, START + 2, a).is_err(),
            "original author no longer holds title"
        );
        assert!(
            prepare_withdraw(&mut fresh, GOD, START + 3, a).is_err(),
            "new title holder is outside this personal-author producer"
        );
    }
    for at in 0..3 {
        let mut world = PlayerWorld::boot_withdrawal(f, granted)?;
        let before = disk(&world);
        let freeze = |world: &PlayerWorld| -> TestResult {
            world.engine.registry.freeze_mechanics_owner(
                &OwnerId::new("economy")?,
                mush_core::mechanics::IncidentId(87),
                mush_core::mechanics::RecoveryReason::IntegrityViolation {
                    code: mush_core::mechanics::CanonicalId::new("withdrawal_fixture")?,
                },
            )?;
            Ok(())
        };
        if at == 0 {
            freeze(&world)?;
            assert!(prepare_withdraw(&mut world, BUYER, START + 4, a).is_err());
        } else {
            let (prepared, output) = prepare_withdraw(&mut world, BUYER, START + 4, a)?;
            if at == 1 {
                freeze(&world)?;
                assert!(stage_withdraw(&mut world, START + 4, a, prepared, output).is_err());
            } else {
                stage_withdraw(&mut world, START + 4, a, prepared, output)?;
                freeze(&world)?;
                assert!(world.engine.take_staged_action().is_err());
            }
        }
        assert!(
            disk(&world) == before,
            "frozen Economy cannot publish withdrawal"
        );
    }
    Ok(())
}
fn cold_corruptions(
    f: &PurchaseFixture,
    saved: &Durable,
    a: &EntitlementAllocationV1,
) -> TestResult {
    for remove in ["cause", "status", "root", "cause_and_root"] {
        let mut lost = saved.clone();
        let before = lost.disk.len();
        lost.disk.retain(|(ns, key), v| {
            if ns != "roll" {
                return true;
            }
            let cause = v.pointer("/family/id") == Some(&json!(CAUSES));
            let status = v.pointer("/family/id") == Some(&json!(STATUS))
                && v.pointer("/value/filed") == Some(&json!(a.filed()));
            let root = key.starts_with("reference:roll:commercial:v1/author_withdrawal/");
            !(match remove {
                "cause" => cause,
                "status" => status,
                "root" => root,
                _ => cause || root,
            })
        });
        assert!(lost.disk.len() < before);
        assert!(
            PlayerWorld::boot_withdrawal(f, &lost).is_err(),
            "missing withdrawal obligation"
        );
    }
    for field in ["withdrawn_by", "recalled_by"] {
        let mut crossed = saved.clone();
        let mut changed = false;
        for ((ns, _), v) in &mut crossed.disk {
            if ns == "roll"
                && v.pointer("/family/id") == Some(&json!(STATUS))
                && v.pointer("/value/filed") == Some(&json!(a.filed()))
            {
                *v.get_mut("value")
                    .and_then(|v| v.get_mut(field))
                    .ok_or("actual status field")? = if field == "withdrawn_by" {
                    Value::Null
                } else {
                    json!(token(START + 50))
                };
                changed = true;
            }
        }
        assert!(changed);
        assert!(
            PlayerWorld::boot_withdrawal(f, &crossed).is_err(),
            "status/cause mismatch or unauthorized recall"
        );
    }
    for field in ["author", "title_revision", "initial_filing"] {
        let mut crossed = saved.clone();
        let mut changed = false;
        for ((ns, _), v) in &mut crossed.disk {
            if ns == "roll" && v.pointer("/family/id") == Some(&json!(CAUSES)) {
                let replacement = match field {
                    "author" => json!(GOD),
                    "title_revision" => json!(
                        v.pointer("/value/title_revision")
                            .and_then(Value::as_u64)
                            .and_then(|n| n.checked_add(1))
                            .ok_or("actual title revision")?
                    ),
                    _ => json!(token(START + 99)),
                };
                *v.get_mut("value")
                    .and_then(|v| v.get_mut(field))
                    .ok_or("actual cause field")? = replacement;
                changed = true;
            }
        }
        assert!(changed);
        assert!(
            PlayerWorld::boot_withdrawal(f, &crossed).is_err(),
            "cause/root or initial-author mismatch"
        );
    }
    Ok(())
}
fn withdraw_after_lapse(
    f: &PurchaseFixture,
    granted: &Durable,
    owner: LicenceId,
    a: &EntitlementAllocationV1,
) -> TestResult<Durable> {
    let parent: PaidTermRecord = serde_json::from_value(
        rows(&granted.disk, "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|r| r.get("licence") != Some(&json!(a.licence())))
            .ok_or("actual parent term")?,
    )?;
    let mut lapsed = granted.clone();
    lapsed.now = parent.window().grace_end().get();
    let mut world = PlayerWorld::boot_withdrawal(f, &lapsed)?;
    source_answer(&mut world, BUYER, owner, None)?;
    source_answer(&mut world, GOD, a.licence(), Some(a.revision().source()))?;
    let before = disk(&world);
    let money = balances(&world)?;
    let prepared = signed(&mut world, START + 50, a)?;
    assert!(
        disk(&world) == before && balances(&world)? == money,
        "preparation cannot publish withdrawal"
    );
    install(&mut world, prepared)?;
    let after = disk(&world);
    unchanged(&before, &after, a)?;
    assert_eq!(balances(&world)?, money);
    let cause = one(rows(&after, "roll", CAUSES))?;
    assert_eq!(cause.get("author"), Some(&json!(BUYER)));
    assert_eq!(cause.get("revision"), Some(&json!(a.revision())));
    assert_eq!(
        status(&world, a)?.get("withdrawn_by"),
        Some(&json!(token(START + 50)))
    );
    assert_eq!(status(&world, a)?.get("recalled_by"), Some(&Value::Null));
    assert!(
        prepare_withdraw(&mut world, BUYER, START + 51, a).is_err(),
        "withdrawal is permanent, not a second cause"
    );
    source_answer(&mut world, GOD, a.licence(), Some(a.revision().source()))?;
    seat(&mut world)?;
    assert!(
        prepare(
            &mut world,
            "market",
            "list_derived_mind",
            1,
            BUYER,
            START + 52,
            listing_params(owner, "open_resale", 100)
        )
        .is_err()
    );
    let saved = capture(&world, &after);
    let cold = PlayerWorld::boot_withdrawal(f, &saved)?;
    assert!(
        disk(&cold) == after,
        "cold audited withdrawal preserves durable evidence"
    );
    // A disabled producer still audits valid immutable history. Present-day
    // title transfer does not rewrite or invalidate the earlier actual cause.
    let mut disabled = PlayerWorld::boot_entitlements(f, &saved)?;
    transfer(&mut disabled, a)?;
    let transferred = capture(&disabled, &disk(&disabled));
    assert!(PlayerWorld::boot_entitlements(f, &transferred).is_ok());
    cold_corruptions(f, &saved, a)?;
    Ok(saved)
}
fn eligible_withdrawal(
    f: &PurchaseFixture,
    granted: &Durable,
    owner: LicenceId,
    a: &EntitlementAllocationV1,
) -> TestResult<Durable> {
    let mut world = PlayerWorld::boot_withdrawal(f, granted)?;
    // Positive control at the same clock: the later refusal is withdrawal,
    // not an already expired parent or owner term.
    seat(&mut world)?;
    commit(
        &mut world,
        "market",
        "list_derived_mind",
        1,
        BUYER,
        START + 60,
        listing_params(owner, "open_resale", 100),
    )?;
    source_answer(&mut world, BUYER, owner, Some(a.revision().source()))?;
    let before = disk(&world);
    let prepared = signed(&mut world, START + 61, a)?;
    assert!(disk(&world) == before);
    install(&mut world, prepared)?;
    unchanged(&before, &disk(&world), a)?;
    seat(&mut world)?;
    assert!(
        prepare(
            &mut world,
            "market",
            "list_derived_mind",
            1,
            BUYER,
            START + 62,
            listing_params(owner, "open_resale", 100)
        )
        .is_err()
    );
    source_answer(&mut world, GOD, a.licence(), Some(a.revision().source()))?;
    // Withdrawn Pin still belongs to the author too; no recall is implied.
    source_answer(&mut world, BUYER, owner, Some(a.revision().source()))?;
    let saved = capture(&world, &disk(&world));
    assert!(PlayerWorld::boot_withdrawal(f, &saved).is_ok());
    Ok(saved)
}
async fn already_sealed(
    f: &PurchaseFixture,
    granted: &Durable,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut world = PlayerWorld::boot_withdrawal(f, granted)?;
    let tasks = world.tasks.clone();
    let result: TestResult = async {
        world.say(GOD, "SIM TEAM red")?;
        world.say(GOD, "SIM AI 1")?;
        world.say(GOD, "SIM UNIT heavy_mk1")?;
        let selected = world.say(GOD, &format!("SIM MIND filed:{} PROFILE Default", a.licence().get()))?;
        assert!(PlayerWorld::private_text(&selected, GOD).contains("Your slot will carry"));
        world.say(GOD, "SIM START")?;
        assert_eq!(world.state()?, match_::MatchState::Boarding);
        let room = match_::centre_object(&world.engine.world, match_::RED_ROOM_ENTRY).ok_or("red room")?;
        world.engine.world.move_to(GOD, room)?;
        world.say(GOD, "BOARD")?;
        assert_eq!(world.state()?, match_::MatchState::Running);
        let id = match_::centre_of(&world.engine.world).and_then(|c| c.match_id).ok_or("match")?;
        let record = world.record(id)?;
        let unit = record.teams.iter().flat_map(|t| t.pilots.iter()).find_map(|(p,u)| (*p==GOD).then_some(*u)).ok_or("paid unit")?;
        let seal = record.minds.get(&unit).ok_or("actual paid seal")?.seal.clone();
        let certificate = seal.requested.origin.certificate().ok_or("actual certificate")?;
        assert_eq!(certificate.rating_job, a.rating().job());
        assert_eq!(certificate.rating_id, a.rating().id().digest());
        let deadline = Instant::now()+WAIT;
        while !world.snapshots.contains_key(&id) {
            world.pump()?;
            if Instant::now()>=deadline { return Err("pre-withdraw pilot snapshot".into()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let opening = world.snapshots.get(&id).ok_or("opening")?;
        let ground = crate::arena_bridge::ground_of(&world.engine.world, id).ok_or("ground")?;
        let arena = crate::arena::Snapshot::decode(&opening.document, (*ground).clone())?;
        assert_eq!(arena.units.iter().find(|u|u.id==unit).ok_or("pilot")?.controller, crate::arena::Controller::Pilot);
        let prepared = signed(&mut world, START+70, a)?;
        install(&mut world, prepared)?;
        assert_eq!(status(&world,a)?.get("withdrawn_by"),Some(&json!(token(START+70))));
        let anchor = world.snapshots.get(&id).map_or(-1, |s| s.at_ms);
        // The mind cannot have run before this post-withdraw public release.
        world.say(GOD,"EXIT")?;
        let first = world.progressed_snapshot(id,unit,anchor,0,-1).await?;
        let (thinks,landed)=crate::minds::bulk::registry_integration_tests::original_purchase::paid_checkout::assert_captured(&world,id,unit,&seal,&first)?;
        let second=world.progressed_snapshot(id,unit,first.at_ms,thinks,landed).await?;
        crate::minds::bulk::registry_integration_tests::original_purchase::paid_checkout::assert_captured(&world,id,unit,&seal,&second)?;
        Ok(())
    }.await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
pub(super) async fn exercise(
    f: &PurchaseFixture,
    granted: &Durable,
    owner: LicenceId,
    a: &EntitlementAllocationV1,
) -> TestResult {
    fences(f, granted, a)?;
    withdraw_after_lapse(f, granted, owner, a)?;
    story_phase("withdrawal.captured", || already_sealed(f, granted, a))?.await?;
    let withdrawn = eligible_withdrawal(f, granted, owner, a)?;
    // Actual public new Pin/source after withdrawal and causal expiry/EXIT on
    // the retained paid seal; no new certificate or synthetic arena is used.
    story_phase("withdrawal.paid_pin", || {
        super::customer(f, &withdrawn, owner, a)
    })?
    .await
}
