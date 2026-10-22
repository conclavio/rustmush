//! Exact accepted-term revocation over the parent's genuinely certified and paid derivative.
//! These are real signed actions and owner/Core ACK installs, not server writer
//! evidence. No certificate, source, status cause or authorization is fabricated.
use super::*;
use mush_core::{DefinitionDigest, OfferExecutionContext, OfferSubmission};
use mush_org::{OrgStatus, OrgStore};
use mush_roll::commerce::registry_revocation::RegistryRevocationPolicy;
const START: u128 = 25_000_000;
const ACTION: &str = "registry_term_revocation";
const CAUSES: &str = "roll_registry_term_revocations";
const STATUS: &str = "roll_registry_revoked_terms";
const ORG: &str = "RevocationRegistry";
fn params(id: LicenceId) -> Value {
    json!({"licence":id.get().to_string(),"reason":"registry_review"})
}
fn setup(
    f: &PurchaseFixture,
    granted: &Durable,
) -> TestResult<(Durable, RegistryRevocationPolicy)> {
    let mut world = PlayerWorld::boot_entitlements(f, granted)?;
    world.say(GOD, &format!("@org/create {ORG}"))?;
    world.say(GOD, &format!("@org/scope create {ORG}=revocation"))?;
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
        .find(|r| r.name.as_str() == "revocation")
        .ok_or("actual Registry scope")?
        .id;
    world.say(
        GOD,
        &format!(
            "@org/grant #{}={ORG} roll.registry.revoke scope={}",
            BUYER.0, scope.0
        ),
    )?;
    let policy = RegistryRevocationPolicy::new(issuer()?, org, scope, vec![BUYER])?;
    let saved = capture(&world, &disk(&world));
    let mut configured = PlayerWorld::boot_revocation(f, &saved, policy.clone())?;
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
    assert!(
        offers.projected_for_action(&policy.action()?).is_some(),
        "actual Org grant admits configured operator"
    );
    Ok((saved, policy))
}
fn signed(world: &mut PlayerWorld, number: u128, id: LicenceId) -> TestResult<PreparedComposedTxn> {
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
        .ok_or("actual signed Registry revocation offer")?;
    let inv = invocation(number, actor, "roll", params(id))?;
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
            // The signed offer fixes reason; only the public selector is supplied.
            parameters: json!({"licence":id.get().to_string()}),
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
fn status(world: &PlayerWorld, id: LicenceId) -> TestResult<Value> {
    rows(&disk(world), "roll", STATUS)
        .into_iter()
        .find(|v| v.get("licence") == Some(&json!(id)))
        .ok_or_else(|| "actual revoked term".into())
}
fn stage_revocation(
    world: &mut PlayerWorld,
    number: u128,
    id: LicenceId,
    prepared: PreparedComposedTxn,
    output: mush_core::action::PreparedAuthoritativeAction,
) -> TestResult {
    let action = action(world, "roll", ACTION, 1)?;
    let inv = invocation(number, BUYER, "roll", params(id))?;
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
    assert_eq!(
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
    policy: &RegistryRevocationPolicy,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut disabled = PlayerWorld::boot_entitlements(f, saved)?;
    assert!(action(&disabled, "roll", ACTION, 1).is_err());
    assert!(
        prepare(
            &mut disabled,
            "roll",
            ACTION,
            1,
            BUYER,
            START,
            params(a.licence())
        )
        .is_err()
    );
    for after_stage in [false, true] {
        let mut world = PlayerWorld::boot_revocation(f, saved, policy.clone())?;
        assert!(
            prepare(
                &mut world,
                "roll",
                ACTION,
                1,
                GOD,
                START,
                params(a.licence())
            )
            .is_err(),
            "unconfigured staff is not Registry authority"
        );
        let (prepared, output) = prepare(
            &mut world,
            "roll",
            ACTION,
            1,
            BUYER,
            START + 1,
            params(a.licence()),
        )?;
        if after_stage {
            stage_revocation(&mut world, START + 1, a.licence(), prepared, output)?;
            suspend(&mut world)?;
            assert!(world.engine.take_staged_action().is_err());
        } else {
            suspend(&mut world)?;
            assert!(
                stage_revocation(&mut world, START + 1, a.licence(), prepared, output).is_err()
            );
        }
        assert!(rows(&disk(&world), "roll", CAUSES).is_empty());
        assert!(rows(&disk(&world), "roll", STATUS).is_empty());
    }
    // Readiness is a live composed latch, not the mere continued existence of
    // an Org slot or an old generic AuthWitness.
    for after_stage in [false, true] {
        let mut world = PlayerWorld::boot_revocation(f, saved, policy.clone())?;
        let (prepared, output) = prepare(
            &mut world,
            "roll",
            ACTION,
            1,
            BUYER,
            START + 2,
            params(a.licence()),
        )?;
        let freeze = |world: &PlayerWorld| -> TestResult {
            world.engine.registry.freeze_mechanics_owner(
                &OwnerId::new("org")?,
                mush_core::mechanics::IncidentId(94),
                mush_core::mechanics::RecoveryReason::IntegrityViolation {
                    code: mush_core::mechanics::CanonicalId::new("registry_revocation_fixture")?,
                },
            )?;
            Ok(())
        };
        if after_stage {
            stage_revocation(&mut world, START + 2, a.licence(), prepared, output)?;
            freeze(&world)?;
            assert!(world.engine.take_staged_action().is_err());
        } else {
            freeze(&world)?;
            assert!(
                stage_revocation(&mut world, START + 2, a.licence(), prepared, output).is_err()
            );
        }
        assert!(rows(&disk(&world), "roll", CAUSES).is_empty());
    }
    Ok(())
}

fn private_status(w: &mut PlayerWorld, actor: ObjId, id: LicenceId, revoked: bool) -> TestResult {
    let events = w.say(actor, &format!("@mindrevoke/status {}", id.get()))?;
    let text = PlayerWorld::private_text(&events, actor);
    assert!(
        text.contains(if revoked {
            "Registry revocation: your mind licence"
        } else {
            "No Registry revocation is recorded"
        }),
        "actual private disposition"
    );
    for (audience, event) in events {
        if mush_testkit::text_of(&event).contains("Registry revocation") {
            assert!(
                matches!(audience,mush_core::Audience::Actor(who)if who==actor)
                    || matches!(audience, mush_core::Audience::Conn(id) if id == if actor == BUYER { BUYER_CONN } else { OBSERVER_CONN }),
                "private readback audience"
            );
        }
    }
    Ok(())
}
fn notice_tick(world: &mut PlayerWorld) -> TestResult<Events> {
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
fn notices(world: &mut PlayerWorld, owner: LicenceId, paid: LicenceId) -> TestResult {
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..4 {
        let mut emitted = 0;
        for (audience, event) in notice_tick(world)? {
            let text = mush_testkit::text_of(&event);
            if text.starts_with("Registry revocation: your mind licence") {
                emitted += 1;
                let actor = match audience {
                    mush_core::Audience::Actor(actor) => actor,
                    _ => return Err("private revocation notice audience".into()),
                };
                let expected = if actor == BUYER {
                    owner
                } else if actor == GOD {
                    paid
                } else {
                    return Err("unrelated notice recipient".into());
                };
                assert!(
                    text.contains(&format!("licence {} term 1 ", expected.get())),
                    "actual own term notice"
                );
                assert!(seen.insert(actor), "same-session notice is deduplicated");
            }
        }
        assert!(emitted <= 16, "bounded lifecycle emissions");
    }
    assert_eq!(seen, std::collections::BTreeSet::from([BUYER, GOD]));
    Ok(())
}
fn revoke(w: &mut PlayerWorld, id: LicenceId, n: u128) -> TestResult {
    let prepared = signed(w, n, id)?;
    install(w, prepared)
}
fn publication(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRevocationPolicy,
    owner: LicenceId,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut w = PlayerWorld::boot_revocation(f, saved, policy.clone())?;
    source_answer(&mut w, BUYER, owner, Some(a.revision().source()))?;
    source_answer(&mut w, GOD, a.licence(), Some(a.revision().source()))?;
    let before = disk(&w);
    let money = balances(&w)?;
    let prepared = signed(&mut w, START + 10, owner)?;
    assert!(disk(&w) == before, "prepare has no sanction");
    install(&mut w, prepared)?;
    for (k, v) in &before {
        assert!(
            disk(&w).get(k) == Some(v),
            "sanction rewrote existing history"
        );
    }
    assert_eq!(balances(&w)?, money);
    source_answer(&mut w, BUYER, owner, None)?;
    source_answer(&mut w, GOD, a.licence(), Some(a.revision().source()))?;
    assert!(
        prepare(
            &mut w,
            "market",
            "list_derived_mind",
            1,
            BUYER,
            START + 11,
            listing_params(owner, "open_resale", 100)
        )
        .is_err(),
        "revoked issuer cannot list"
    );
    private_status(&mut w, BUYER, owner, true)?;
    private_status(&mut w, GOD, a.licence(), false)?;
    assert!(
        signed(&mut w, START + 12, owner).is_err(),
        "one permanent sanction per accepted generation"
    );
    revoke(&mut w, a.licence(), START + 13)?;
    source_answer(&mut w, GOD, a.licence(), None)?;
    let foreign = w.say(BUYER, &format!("@mindrevoke/status {}", a.licence().get()))?;
    let missing = w.say(BUYER, "@mindrevoke/status 18446744073709551615")?;
    assert_eq!(
        PlayerWorld::private_text(&foreign, BUYER),
        PlayerWorld::private_text(&missing, BUYER)
    );
    notices(&mut w, owner, a.licence())?;
    let retained = capture(&w, &disk(&w));
    let mut cold = PlayerWorld::boot_revocation(f, &retained, policy.clone())?;
    source_answer(&mut cold, BUYER, owner, None)?;
    source_answer(&mut cold, GOD, a.licence(), None)?;
    private_status(&mut cold, GOD, a.licence(), true)?;
    // Revoking current operator authority cannot erase retained sanctions.
    suspend(&mut cold)?;
    let revoked_policy = capture(&cold, &disk(&cold));
    let mut disabled = PlayerWorld::boot_entitlements(f, &revoked_policy)?;
    assert!(action(&disabled, "roll", ACTION, 1).is_err());
    private_status(&mut disabled, GOD, a.licence(), true)?;
    source_answer(&mut disabled, GOD, a.licence(), None)?;
    cold_corruptions(f, &retained, policy)?;
    Ok(())
}
fn cold_corruptions(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRevocationPolicy,
) -> TestResult {
    for remove in ["cause", "status", "root"] {
        let mut broken = saved.clone();
        let before = broken.disk.len();
        let mut removed = false;
        broken.disk.retain(|(ns, key), v| {
            if removed || ns != "roll" {
                return true;
            }
            let hit = match remove {
                "cause" => v.pointer("/family/id") == Some(&json!(CAUSES)),
                "status" => v.pointer("/family/id") == Some(&json!(STATUS)),
                _ => key.starts_with("reference:roll:commercial:v1/registry_revocation/"),
            };
            if hit {
                removed = true;
            }
            !hit
        });
        assert_eq!(broken.disk.len() + 1, before);
        assert!(
            PlayerWorld::boot_revocation(f, &broken, policy.clone()).is_err(),
            "missing paired {remove}"
        );
    }
    for field in ["generation", "term_digest", "operation"] {
        let mut broken = saved.clone();
        let row = broken
            .disk
            .values_mut()
            .find(|v| v.pointer("/family/id") == Some(&json!(STATUS)))
            .ok_or("status")?;
        let value = row
            .get_mut("value")
            .and_then(Value::as_object_mut)
            .ok_or("value")?;
        value.insert(
            field.into(),
            match field {
                "generation" => json!("2"),
                "operation" => json!(token(START + 999)),
                _ => json!(DefinitionDigest::from_bytes([0xfe; 32])),
            },
        );
        assert!(
            PlayerWorld::boot_revocation(f, &broken, policy.clone()).is_err(),
            "crossed term proof {field}"
        );
    }
    Ok(())
}
fn parent_independence(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRevocationPolicy,
    owner: LicenceId,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut w = PlayerWorld::boot_revocation(f, saved, policy.clone())?;
    let parent: PaidTermRecord = serde_json::from_value(
        rows(&disk(&w), "roll", "roll_mind_paid_term")
            .into_iter()
            .find(|v| v.get("operation") != Some(&json!(a.operation())))
            .ok_or("actual acquired parent")?,
    )?;
    revoke(&mut w, parent.licence(), START + 20)?;
    source_answer(&mut w, BUYER, parent.licence(), None)?;
    source_answer(&mut w, BUYER, owner, None)?;
    source_answer(&mut w, GOD, a.licence(), Some(a.revision().source()))?;
    let retained = capture(&w, &disk(&w));
    let mut cold = PlayerWorld::boot_revocation(f, &retained, policy.clone())?;
    source_answer(&mut cold, BUYER, owner, None)?;
    source_answer(&mut cold, GOD, a.licence(), Some(a.revision().source()))?;
    Ok(())
}
fn late_consumer_fence(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRevocationPolicy,
    owner: LicenceId,
) -> TestResult {
    for after_stage in [false, true] {
        let mut w = PlayerWorld::boot_revocation(f, saved, policy.clone())?;
        let before = disk(&w);
        let p = listing_params(owner, "open_resale", 100);
        let (prepared, out) = prepare(
            &mut w,
            "market",
            "list_derived_mind",
            1,
            BUYER,
            START + 22,
            p.clone(),
        )?;
        let exact = action(&w, "market", "list_derived_mind", 1)?;
        // Core admits one pending commit: a sanction cannot land between a
        // prepared (or staged) listing and its install, so the revocation is
        // refused and nothing is written. The opposite order (sanction first,
        // then the listing refused) is covered by the story's own flow.
        if after_stage {
            stage(&mut w, exact, BUYER, START + 22, p, prepared, out)?;
            assert!(
                revoke(&mut w, owner, START + 23).is_err(),
                "sanction landed under a staged listing"
            );
        } else {
            assert!(
                revoke(&mut w, owner, START + 23).is_err(),
                "sanction landed under a prepared listing"
            );
            drop((exact, p, prepared, out));
        }
        assert!(disk(&w) == before, "refused sanction changed retained rows");
    }
    Ok(())
}
fn original_owner_available(w: &mut PlayerWorld, owner: LicenceId, expected: bool) -> TestResult {
    let events = w.say(GOD, "SIM MIND /filed")?;
    let text = PlayerWorld::private_text(&events, GOD);
    assert_eq!(
        text.contains(&format!("filed:{}:", owner.get())),
        expected,
        "actual current original-owner projection"
    );
    Ok(())
}
fn new_owner_term(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRevocationPolicy,
) -> TestResult {
    let mut w = PlayerWorld::boot_revocation(f, saved, policy.clone())?;
    // This is the genuine original author's first filing, verified before using its ID.
    let owner = LicenceId::new(1)?;
    let birth: InitialFilingOperation = serde_json::from_value(
        rows(&disk(&w), "roll", "roll_mind_initial_filing")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(owner)))
            .ok_or("original birth")?,
    )?;
    let birth_wire = serde_json::to_value(&birth)?;
    assert_eq!(birth_wire.get("requester"), Some(&json!(GOD)));
    assert!(birth_wire.get("parent").is_none());
    let term: mush_roll::commerce::records::OwnerTerm = serde_json::from_value(
        rows(&disk(&w), "roll", "roll_mind_owner_term")
            .into_iter()
            .find(|v| v.get("licence") == Some(&json!(owner)))
            .ok_or("original term")?,
    )?;
    original_owner_available(&mut w, owner, true)?;
    revoke(&mut w, owner, START + 30)?;
    original_owner_available(&mut w, owner, false)?;
    for at in [
        term.window().term_end().get(),
        term.window().grace_end().get() - 1,
    ] {
        w.engine.world.set_now(at);
        assert_eq!(w.engine.world.world_ic_now().at, at, "actual WorldIC clock");
        assert!(
            prepare(
                &mut w,
                "roll",
                "attest_owner_mind",
                1,
                GOD,
                START + 31,
                json!({"owner_licence":owner.get()})
            )
            .is_err(),
            "sanction includes original Grace"
        );
    }
    let historical = status(&w, owner)?;
    w.engine.world.set_now(term.window().grace_end().get());
    commit(
        &mut w,
        "roll",
        "attest_owner_mind",
        1,
        GOD,
        START + 32,
        json!({"owner_licence":owner.get()}),
    )?;
    let current = rows(&disk(&w), "roll", "roll_mind_owner_licence")
        .into_iter()
        .find(|v| v.get("id") == Some(&json!(owner)))
        .ok_or("current owner")?;
    assert_eq!(current.get("term"), Some(&json!("2")));
    assert!(
        status(&w, owner)? == historical,
        "new term erased sanction history"
    );
    original_owner_available(&mut w, owner, true)?;
    private_status(&mut w, GOD, owner, false)?;
    let old = w.say(GOD, &format!("@mindrevoke/status {}/1", owner.get()))?;
    assert!(
        PlayerWorld::private_text(&old, GOD).contains("Registry revocation: your mind licence")
    );
    let saved = capture(&w, &disk(&w));
    let mut cold = PlayerWorld::boot_revocation(f, &saved, policy.clone())?;
    original_owner_available(&mut cold, owner, true)?;
    Ok(())
}
fn grace_admission(
    f: &PurchaseFixture,
    saved: &Durable,
    policy: &RegistryRevocationPolicy,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut w = PlayerWorld::boot_revocation(f, saved, policy.clone())?;
    let term = own_term(&w, a.licence())?;
    w.engine.world.set_now(term.window().term_end().get());
    revoke(&mut w, a.licence(), START + 40)?;
    assert_eq!(
        status(&w, a.licence())?.get("window"),
        Some(&serde_json::to_value(term.window())?)
    );
    source_answer(&mut w, GOD, a.licence(), None)?;
    // A rollback cannot restore use of that accepted generation.
    w.engine.world.set_now(term.window().issued_at().get());
    source_answer(&mut w, GOD, a.licence(), None)?;
    let mut expired = PlayerWorld::boot_revocation(f, saved, policy.clone())?;
    expired
        .engine
        .world
        .set_now(term.window().grace_end().get());
    assert!(
        signed(&mut expired, START + 41, a.licence()).is_err(),
        "an already expired term is not a current sanction target"
    );
    assert!(rows(&disk(&expired), "roll", CAUSES).is_empty());
    Ok(())
}
async fn already_sealed(
    f: &PurchaseFixture,
    granted: &Durable,
    policy: &RegistryRevocationPolicy,
    a: &EntitlementAllocationV1,
) -> TestResult {
    let mut world = PlayerWorld::boot_revocation(f, granted, policy.clone())?;
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
            if Instant::now()>=deadline { return Err("pre-revocation pilot snapshot".into()); }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let opening = world.snapshots.get(&id).ok_or("opening")?;
        let ground = crate::arena_bridge::ground_of(&world.engine.world, id).ok_or("ground")?;
        let arena = crate::arena::Snapshot::decode(&opening.document, (*ground).clone())?;
        assert_eq!(arena.units.iter().find(|u|u.id==unit).ok_or("pilot")?.controller, crate::arena::Controller::Pilot);
        let prepared = signed(&mut world, START+70, a.licence())?;
        install(&mut world, prepared)?;
        assert_eq!(status(&world,a.licence())?.get("operation"),Some(&json!(token(START+70))));
        let anchor = world.snapshots.get(&id).map_or(-1, |s| s.at_ms);
        // The mind cannot have run before this post-revocation public release.
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
    parent_independence(f, &saved, &policy, owner, a)?;
    late_consumer_fence(f, &saved, &policy, owner)?;
    new_owner_term(f, &saved, &policy)?;
    grace_admission(f, &saved, &policy, a)?;
    story_phase("registry.revocation.captured", || {
        already_sealed(f, &saved, &policy, a)
    })?
    .await
}
