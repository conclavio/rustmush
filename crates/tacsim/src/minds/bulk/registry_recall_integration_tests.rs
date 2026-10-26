//! Exact filed recall over the parent's genuinely certified and paid derivative.
//! These are real signed actions and owner/Core ACK installs, not server writer
//! evidence. No certificate, source, status cause or authorization is fabricated.
use super::*;
use mush_core::{OfferExecutionContext, OfferSubmission};
use mush_org::{OrgStatus, OrgStore};
use mush_roll::commerce::registry_recall::RegistryRecallPolicy;
const START: u128 = 19_000_000;
const ACTION: &str = "registry_revision_recall";
const CAUSES: &str = "roll_registry_revision_recalls";
const STATUS: &str = "roll_mind_status";
const ORG: &str = "RecallRegistry";
fn params(a: &EntitlementAllocationV1) -> Value {
    json!({"filed":a.filed().get().to_string(),"reason":"registry_review"})
}
fn setup(f: &PurchaseFixture, granted: &Durable) -> TestResult<(Durable, RegistryRecallPolicy)> {
    let mut world = PlayerWorld::boot_entitlements(f, granted)?;
    world.say(GOD, &format!("@org/create {ORG}"))?;
    world.say(GOD, &format!("@org/scope create {ORG}=recall"))?;
    let state = world
        .engine
        .world
        .module_state::<OrgStore>()
        .ok_or("actual Org")?
        .state();
    let org = state
        .orgs()
        .find(|r| r.name.as_str() == ORG)
        .ok_or("actual Registry org")?
        .id;
    let scope = state
        .scopes_in(org)
        .find(|r| r.name.as_str() == "recall")
        .ok_or("actual Registry scope")?
        .id;
    world.say(
        GOD,
        &format!(
            "@org/grant #{}={ORG} roll.registry.recall scope={}",
            BUYER.0, scope.0
        ),
    )?;
    let policy = RegistryRecallPolicy::new(issuer()?, org, scope, vec![BUYER])?;
    let saved = capture(&world, &disk(&world));
    let mut configured = PlayerWorld::boot_recall(f, &saved, policy.clone())?;
    configured
        .engine
        .set_offer_keys(mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(1, [0x57; 32])?,
            [],
        )?);
    let query = configured.engine.legal_action_query(
        PrincipalId::Object { id: BUYER },
        BUYER,
        PerceivedRef::SelfActor,
    )?;
    let offers = configured.engine.project_legal_actions(&query)?;
    check!(
        offers.projected_for_action(&policy.action()?).is_some(),
        "actual Org grant admits configured operator"
    );
    Ok((saved, policy))
}
fn signed(
    world: &mut PlayerWorld,
    number: u128,
    a: &EntitlementAllocationV1,
) -> TestResult<PreparedComposedTxn> {
    world.engine.set_offer_keys(mush_core::OfferKeyRing::new(
        mush_core::OfferSecret::new(1, [0x57; 32])?,
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
        .ok_or("actual signed Registry recall offer")?;
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
            // Reason is fixed by the signed offer; submit only its public field.
            parameters: json!({"filed":a.filed().get().to_string()}),
        },
    )?;
    let (exact, resolved, _) = resolved.into_authoritative_parts();
    check_eq!(exact, action);
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
fn stage_recall(
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
        ingress_hash: CanonicalHash::from_bytes([101; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([102; 32]),
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
fn suspend(world: &mut PlayerWorld) -> TestResult {
    world.say(GOD, &format!("@org/status {ORG}=suspended"))?;
    check_eq!(
        world
            .engine
            .world
            .module_state::<OrgStore>()
            .ok_or("Org")?
            .state()
            .orgs()
            .find(|r| r.name.as_str() == ORG)
            .ok_or("Registry")?
            .status,
        OrgStatus::Suspended
    );
    Ok(())
}
fn fences(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRecallPolicy,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut disabled = PlayerWorld::boot_entitlements(f, saved)?;
    check!(action(&disabled, "roll", ACTION, 1).is_err());
    check!(crate::test_checks::refused(prepare(
        &mut disabled,
        "roll",
        ACTION,
        1,
        BUYER,
        START,
        params(a)
    ))?);
    for after_stage in [false, true] {
        let mut world = PlayerWorld::boot_recall(f, saved, policy.clone())?;
        check!(
            crate::test_checks::refused(prepare(
                &mut world,
                "roll",
                ACTION,
                1,
                GOD,
                START,
                params(a)
            ))?,
            "unconfigured staff is not Registry authority"
        );
        let (prepared, output) =
            prepare(&mut world, "roll", ACTION, 1, BUYER, START + 1, params(a))?;
        if after_stage {
            stage_recall(&mut world, START + 1, a, prepared, output)?;
            suspend(&mut world)?;
            check!(world.engine.take_staged_action().is_err());
        } else {
            suspend(&mut world)?;
            check!(stage_recall(&mut world, START + 1, a, prepared, output).is_err());
        }
        check!(rows(&disk(&world), "roll", CAUSES).is_empty());
        check_eq!(status(&world, a)?.get("recalled_by"), Some(&Value::Null));
    }
    // Readiness is a live composed latch, not the mere continued existence of
    // an Org slot or an old generic AuthWitness.
    for after_stage in [false, true] {
        let mut world = PlayerWorld::boot_recall(f, saved, policy.clone())?;
        let (prepared, output) =
            prepare(&mut world, "roll", ACTION, 1, BUYER, START + 2, params(a))?;
        let freeze = |world: &PlayerWorld| -> TestResult {
            world.engine.registry.freeze_mechanics_owner(
                &OwnerId::new("org")?,
                mush_core::mechanics::IncidentId(94),
                mush_core::mechanics::RecoveryReason::IntegrityViolation {
                    code: mush_core::mechanics::CanonicalId::new("registry_recall_fixture")?,
                },
            )?;
            Ok(())
        };
        if after_stage {
            stage_recall(&mut world, START + 2, a, prepared, output)?;
            freeze(&world)?;
            check!(world.engine.take_staged_action().is_err());
        } else {
            freeze(&world)?;
            check!(stage_recall(&mut world, START + 2, a, prepared, output).is_err());
        }
        check!(rows(&disk(&world), "roll", CAUSES).is_empty());
    }
    Ok(())
}
fn private_status(
    world: &mut PlayerWorld,
    actor: ObjId,
    licence: LicenceId,
    recalled: bool,
) -> TestResult {
    let events = world.say(actor, &format!("@mindrecall/status {}", licence.get()))?;
    let text = PlayerWorld::private_text(&events, actor);
    check!(
        text.contains(if recalled {
            "Registry recall: your mind licence"
        } else {
            "No Registry recall is recorded"
        }),
        "actual holder readback disposition"
    );
    let conn = if actor == BUYER {
        BUYER_CONN
    } else {
        OBSERVER_CONN
    };
    for (a, e) in events {
        if mush_testkit::text_of(&e).contains("Registry recall") {
            check!(
                matches!(a,mush_core::Audience::Actor(who)if who==actor)
                    || matches!(a,mush_core::Audience::Conn(id)if id==conn),
                "recall readback remains private"
            );
        }
    }
    Ok(())
}
fn tick(world: &mut PlayerWorld) -> TestResult<Events> {
    let now = world
        .engine
        .world
        .now()
        .checked_add(5)
        .ok_or("notice clock")?;
    world.engine.world.set_now(now);
    let mut out = Outbox::new();
    world.engine.fire_due(
        now.checked_mul(1000).ok_or("notice milliseconds")?,
        &mut out,
        &mut mush_core::Effects::new(),
    );
    let events = out.take();
    world.commit()?;
    Ok(events)
}
fn notices(world: &mut PlayerWorld, owner: LicenceId, a: &EntitlementAllocationV1) -> TestResult {
    private_status(world, BUYER, owner, true)?;
    private_status(world, GOD, a.licence(), true)?;
    let foreign = world.say(BUYER, &format!("@mindrecall/status {}", a.licence().get()))?;
    let missing = world.say(BUYER, "@mindrecall/status 18446744073709551615")?;
    check_eq!(
        PlayerWorld::private_text(&foreign, BUYER),
        PlayerWorld::private_text(&missing, BUYER),
        "foreign and missing private holdings share refusal"
    );
    check!(!PlayerWorld::private_text(&foreign, BUYER).contains(&token(START + 10).to_hex()));
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..4 {
        for (audience, event) in tick(world)? {
            let text = mush_testkit::text_of(&event);
            if text.starts_with("Registry recall: your mind licence") {
                let actor = match audience {
                    mush_core::Audience::Actor(actor) => actor,
                    _ => return Err("private notice audience".into()),
                };
                check!(actor == BUYER || actor == GOD);
                check!(seen.insert(actor), "no same-session repeated notice");
            }
        }
    }
    check_eq!(seen, std::collections::BTreeSet::from([BUYER, GOD]));
    Ok(())
}
fn unchanged(before: &Disk, after: &Disk, a: &EntitlementAllocationV1) -> TestResult {
    for (k, v) in before {
        if v.pointer("/family/id") == Some(&json!(STATUS))
            && v.pointer("/value/filed") == Some(&json!(a.filed()))
        {
            continue;
        }
        check!(
            after.get(k) == Some(v),
            "recall changed unrelated historical evidence"
        );
    }
    Ok(())
}
fn cold_corruptions(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRecallPolicy,
    a: &EntitlementAllocationV1,
) -> TestResult {
    for remove in ["cause", "status", "root", "cause_and_root"] {
        let mut lost = saved.clone();
        let len = lost.disk.len();
        lost.disk.retain(|(ns, key), v| {
            if ns != "roll" {
                return true;
            }
            let cause = v.pointer("/family/id") == Some(&json!(CAUSES));
            let status = v.pointer("/family/id") == Some(&json!(STATUS))
                && v.pointer("/value/filed") == Some(&json!(a.filed()));
            let root = key.starts_with("reference:roll:commercial:v1/registry_recall/");
            !match remove {
                "cause" => cause,
                "status" => status,
                "root" => root,
                _ => cause || root,
            }
        });
        check!(lost.disk.len() < len);
        check!(
            crate::test_checks::refused(PlayerWorld::boot_recall(f, &lost, policy.clone()))?,
            "missing exact recall obligation"
        );
    }
    for field in ["actor", "filed", "initial_operation", "action"] {
        let mut crossed = saved.clone();
        let mut changed = false;
        for ((ns, _), v) in &mut crossed.disk {
            if ns == "roll" && v.pointer("/family/id") == Some(&json!(CAUSES)) {
                let value = v.get_mut("value").ok_or("actual cause value")?;
                let replacement = match field {
                    "actor" => json!(GOD),
                    "filed" => json!(u64::MAX.to_string()),
                    "initial_operation" => json!(token(START + 999)),
                    _ => serde_json::to_value(policy.action()?)?,
                };
                if field == "action" {
                    let current = value.get_mut("action").ok_or("actual action")?;
                    // Keep a structurally valid full reference but cross its version.
                    *current.get_mut("version").ok_or("action version")? = json!(2);
                } else {
                    *value.get_mut(field).ok_or("actual cause field")? = replacement;
                }
                changed = true;
            }
        }
        check!(changed);
        check!(
            crate::test_checks::refused(PlayerWorld::boot_recall(f, &crossed, policy.clone()))?,
            "crossed cause cannot authenticate actual status/root"
        );
    }
    Ok(())
}
fn publication(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRecallPolicy,
    owner: LicenceId,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut world = PlayerWorld::boot_recall(f, saved, policy.clone())?;
    source_answer(&mut world, BUYER, owner, Some(a.revision().source()))?;
    source_answer(&mut world, GOD, a.licence(), Some(a.revision().source()))?;
    private_status(&mut world, BUYER, owner, false)?;
    private_status(&mut world, GOD, a.licence(), false)?;
    let before = disk(&world);
    let money = balances(&world)?;
    let prepared = signed(&mut world, START + 10, a)?;
    check!(disk(&world) == before);
    private_status(&mut world, GOD, a.licence(), false)?;
    install(&mut world, prepared)?;
    let after = disk(&world);
    unchanged(&before, &after, a)?;
    check_eq!(balances(&world)?, money);
    let cause = one(rows(&after, "roll", CAUSES))?;
    check_eq!(cause.get("filed"), Some(&json!(a.filed())));
    check_eq!(cause.get("rating"), Some(&json!(a.rating())));
    check_eq!(
        status(&world, a)?.get("recalled_by"),
        Some(&json!(token(START + 10)))
    );
    check_eq!(status(&world, a)?.get("withdrawn_by"), Some(&Value::Null));
    check!(
        crate::test_checks::refused(prepare(
            &mut world,
            "roll",
            ACTION,
            1,
            BUYER,
            START + 11,
            params(a)
        ))?,
        "no duplicate recall cause"
    );
    source_answer(&mut world, BUYER, owner, None)?;
    source_answer(&mut world, GOD, a.licence(), None)?;
    // BUYER lists from the market venue; GOD stays in the lobby for SIM below.
    world.engine.world.move_to(BUYER, ROOM)?;
    world.commit()?;
    check!(crate::test_checks::refused(prepare(
        &mut world,
        "market",
        "list_derived_mind",
        1,
        BUYER,
        START + 12,
        listing_params(owner, "open_resale", 100)
    ))?);
    let parent: PaidTermRecord = serde_json::from_value(
        rows(&before, "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|v| v.get("operation") != Some(&json!(a.operation())))
            .ok_or("actual immediate parent term")?,
    )?;
    let mush_roll::commerce::records::RevisionPolicy::Pin { revision, .. } = parent.policy() else {
        return Err("actual parent Pin".into());
    };
    source_answer(&mut world, BUYER, parent.licence(), Some(revision.source()))?;
    // New paid checkout is blocked by exact recall, while the separate branch
    // below proves the already captured native seal continues.
    world.say(GOD, "SIM TEAM red")?;
    world.say(GOD, "SIM UNIT heavy_mk1")?;
    let refused = world.say(
        GOD,
        &format!("SIM MIND filed:{} PROFILE Default", a.licence().get()),
    )?;
    check!(
        !PlayerWorld::private_text(&refused, GOD).contains("Your slot will carry"),
        "recalled paid revision has no new admission"
    );
    notices(&mut world, owner, a)?;
    let retained = capture(&world, &disk(&world));
    let mut cold = PlayerWorld::boot_recall(f, &retained, policy.clone())?;
    private_status(&mut cold, GOD, a.licence(), true)?;
    // Current policy revocation must stop new authority while preserving the
    // immutable cause and private historical-holder readback.
    suspend(&mut cold)?;
    let revoked = capture(&cold, &disk(&cold));
    let mut disabled = PlayerWorld::boot_entitlements(f, &revoked)?;
    private_status(&mut disabled, GOD, a.licence(), true)?;
    check!(action(&disabled, "roll", ACTION, 1).is_err());
    cold_corruptions(f, &retained, policy, a)?;
    Ok(())
}
fn withdrawn_then_recalled(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRecallPolicy,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut world = PlayerWorld::boot_recall(f, saved, policy.clone())?;
    commit(
        &mut world,
        "roll",
        "withdraw_authored_mind",
        1,
        BUYER,
        START + 60,
        json!({"filed":a.filed().get().to_string()}),
    )?;
    let before = disk(&world);
    source_answer(&mut world, GOD, a.licence(), Some(a.revision().source()))?;
    let prepared = signed(&mut world, START + 61, a)?;
    install(&mut world, prepared)?;
    unchanged(&before, &disk(&world), a)?;
    let row = status(&world, a)?;
    check_eq!(row.get("withdrawn_by"), Some(&json!(token(START + 60))));
    check_eq!(row.get("recalled_by"), Some(&json!(token(START + 61))));
    source_answer(&mut world, GOD, a.licence(), None)?;
    let saved = capture(&world, &disk(&world));
    let mut cold = PlayerWorld::boot_entitlements(f, &saved)?;
    private_status(&mut cold, GOD, a.licence(), true)?;
    Ok(())
}
fn bounded_notice_drain(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRecallPolicy,
    owner: LicenceId,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut world = PlayerWorld::boot_recall(f, saved, policy.clone())?;
    // Each recipient holding below is issued by the actual six-target E4/Roll/
    // Market Grant. No synthetic licence rows are used to fill the queue.
    cash(&mut world)?;
    for actor in [GOD, BUYER] {
        world.engine.world.move_to(actor, ROOM)?;
    }
    list(&mut world, owner, START + 100)?;
    let mut expected =
        std::collections::BTreeSet::from([(BUYER, owner.get()), (GOD, a.licence().get())]);
    for i in 0..18u128 {
        let at = START + 110 + i * 3;
        quote(&mut world, START + 100, at)?;
        fund(&mut world, at, at + 1)?;
        grant(&mut world, at + 1, at + 2)?;
        let allocation = allocation(&world, at + 1)?;
        check!(expected.insert((GOD, allocation.licence().get())));
    }
    let prepared = signed(&mut world, START + 200, a)?;
    install(&mut world, prepared)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut first = 0;
    for round in 0..8 {
        let mut count = 0;
        for (audience, event) in tick(&mut world)? {
            let text = mush_testkit::text_of(&event);
            if let Some(rest) = text.strip_prefix("Registry recall: your mind licence ") {
                let licence = rest
                    .split_whitespace()
                    .next()
                    .ok_or("notice licence")?
                    .parse::<u64>()?;
                let actor = match audience {
                    mush_core::Audience::Actor(actor) => actor,
                    _ => return Err("notice audience".into()),
                };
                check!(
                    expected.contains(&(actor, licence)),
                    "notice has exact actual holder"
                );
                check!(
                    seen.insert((actor, licence)),
                    "same session deduplicates actual holdings"
                );
                count += 1;
            }
        }
        check!(count <= 16, "real periodic drain emission bound");
        if round == 0 {
            first = count;
        }
    }
    check!(
        first > 0 && first < expected.len(),
        "twenty real holdings require more than one bounded drain"
    );
    check!(
        seen == expected,
        "both inventories make bounded progress without starvation"
    );
    Ok(())
}
async fn already_sealed(
    f: &PurchaseFixture,
    granted: &Durable,
    policy: &RegistryRecallPolicy,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut world = PlayerWorld::boot_recall(f, granted, policy.clone())?;
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
            if Instant::now()>=deadline { return Err("pre-recall pilot snapshot".into()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let opening = world.snapshots.get(&id).ok_or("opening")?;
        let ground = crate::arena_bridge::ground_of(&world.engine.world, id).ok_or("ground")?;
        let arena = crate::arena::Snapshot::decode(&opening.document, (*ground).clone())?;
        assert_eq!(arena.units.iter().find(|u|u.id==unit).ok_or("pilot")?.controller, crate::arena::Controller::Pilot);
        let prepared = signed(&mut world, START+70, a)?;
        install(&mut world, prepared)?;
        assert_eq!(status(&world,a)?.get("recalled_by"),Some(&json!(token(START+70))));
        let anchor = world.snapshots.get(&id).map_or(-1, |s| s.at_ms);
        // The mind cannot have run before this post-recall public release.
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
    let (saved, policy) = setup(f, granted)?;
    fences(f, &saved, &policy, a)?;
    publication(f, &saved, &policy, owner, a)?;
    withdrawn_then_recalled(f, &saved, &policy, a)?;
    bounded_notice_drain(f, &saved, &policy, owner, a)?;
    story_phase("registry.recall.captured", || {
        already_sealed(f, &saved, &policy, a)
    })?
    .await
}
