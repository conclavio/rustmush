//! Real port, existing NPC, command ingress, coordinator, acknowledgement and boot.
use super::dt_unit_authority::{persist_legacy, stage, submit};
use mush_core::{Attr, CanonicalId, Engine, GOD, ObjId, ObjKind, OfferKeyRing, OfferSecret};
use mush_server::action_coordinator::{ActionCoordinator, ActionPreflight};
use mush_store::{AuthoritativeStore, InMemoryStore};
use mush_testkit::{TestWorld, text_of};
use std::collections::BTreeSet;
use std::sync::Arc;
use tacsim::detachment::crew::{
    CrewContract, CrewDuty, CrewHandles, HireTerms, NpcMissionScope, OFFER_ATTR,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
struct Rig {
    world: TestWorld,
    employer: ObjId,
    competitor: ObjId,
    npc: ObjId,
    room: ObjId,
    away: ObjId,
}
fn terms() -> TestResult<HireTerms> {
    Ok(HireTerms {
        schema: 1,
        port: CanonicalId::new("ceres")?,
        mission_scope: NpcMissionScope::SubordinateOperator,
        duties: BTreeSet::from([CrewDuty::Mining, CrewDuty::Recovery]),
    })
}
async fn fixture(store: &InMemoryStore) -> TestResult<Rig> {
    let mut config = mush_config::config::Config::default();
    config.tacsim.enabled = true;
    let llm = Arc::new(mush_llm::LlmService::from_config(
        &mush_llm::LlmConfig::default(),
    )?);
    let modules = mush_server::boot::game_modules(&config, &llm)?;
    let world = mush_server::persist::load_or_seed_world(store).await?;
    let mut engine =
        Engine::with_modules(world, mush_server::world_task::base_registry(), modules)?;
    mush_server::modules::boot_modules(&mut engine, store).await?;
    engine.world.set_now(100);
    engine.set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x64; 32])?, [])?);
    let room_key = mush_atlas::RecordKey::new(
        CanonicalId::new("neighborhood.ceres_freeport")?,
        CanonicalId::new("room.c10_slack_line")?,
    );
    let room =
        tacsim::port::apply::room_by_key(&engine.world, &room_key).ok_or("real Ceres crew bar")?;
    let mut ensure = |name: &str, kind, at| -> TestResult<ObjId> {
        let known = engine.world.iter().find(|o| o.name == name).map(|o| o.id);
        Ok(match known {
            Some(id) => id,
            None => engine.world.create(kind, name, GOD, at)?,
        })
    };
    let employer = ensure("Crew employer", ObjKind::Player, Some(room))?;
    let competitor = ensure("Crew competing employer", ObjKind::Player, Some(room))?;
    let npc = ensure("Crew mining operator", ObjKind::Npc, Some(room))?;
    let away = ensure("Crew boarding destination", ObjKind::Room, None)?;
    let exit = ensure("crew-test-board", ObjKind::Exit, Some(room))?;
    engine.world.link_exit(exit, Some(away))?;
    if engine
        .world
        .get(npc)
        .is_none_or(|o| !o.attrs.contains(OFFER_ATTR))
    {
        engine.world.set_attr(
            npc,
            OFFER_ATTR,
            Attr::json(serde_json::to_value(terms()?)?, GOD),
        )?;
    }
    let mut world = TestWorld::from_engine(engine);
    persist_legacy(&mut world, store).await?;
    Ok(Rig {
        world,
        employer,
        competitor,
        npc,
        room,
        away,
    })
}
fn contract(rig: &Rig) -> TestResult<Option<CrewContract>> {
    let handles = rig
        .world
        .world()
        .module_state::<CrewHandles>()
        .ok_or("crew handles")?;
    Ok(rig
        .world
        .engine()
        .action_state()
        .get(&handles.contracts, &rig.npc)?
        .cloned())
}
fn hire(npc: ObjId, terms: &HireTerms) -> TestResult<String> {
    Ok(format!(
        "+sortie/hire #{}={}",
        npc.0,
        terms.digest()?.to_hex()
    ))
}
fn text(world: &mut TestWorld, actor: ObjId, line: &str) -> String {
    world
        .run(actor, line)
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}
async fn commit_line(rig: &mut Rig, store: &InMemoryStore, actor: ObjId, line: &str) -> TestResult {
    let request = submit(&mut rig.world, actor, line)?;
    stage(&mut rig.world, store, &request)
        .await
        .map_err(|e| format!("{line}: {e}"))?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged crew setup")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    rig.world
        .engine_mut()
        .install_composed_action(installation)?;
    Ok(())
}
#[tokio::test]
async fn hiring_survives_lost_ack_and_restart_without_boarding_or_financial_writes() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let employer = rig.employer;
    let npc = rig.npc;
    let listed = text(&mut rig.world, employer, "+sortie/crew");
    assert!(listed.contains("Crew mining operator"), "{listed}");
    assert!(listed.contains(&hire(npc, &terms()?)?));
    let request = submit(&mut rig.world, employer, &hire(npc, &terms()?)?)?;
    let ingress = stage(&mut rig.world, &store, &request).await?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged hire")?
        .into_parts();
    assert!(contract(&rig)?.is_none());
    assert_eq!(
        bundle.writes().len(),
        1,
        "hiring writes only its one contract"
    );
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert!(
        contract(&rig)?.is_none(),
        "durability without acknowledgement does not publish live employment"
    );
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(rig.room)
    );
    drop(installation);
    drop(rig);
    let mut rig = fixture(&store).await?;
    let agreement = contract(&rig)?.ok_or("recovered contract")?;
    assert_eq!(agreement.employer, employer);
    assert_eq!(agreement.terms, terms()?);
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(rig.room)
    );
    let ActionPreflight::Finalized(retry) = ActionCoordinator::preflight(&store, ingress).await?
    else {
        return Err("hire retry was not finalized".into());
    };
    assert_eq!(retry.outcome(), receipt.outcome());
    assert!(!text(&mut rig.world, employer, "+sortie/crew").contains("Crew mining operator"));
    let shown = text(
        &mut rig.world,
        employer,
        &format!("+sortie/contract #{}", npc.0),
    );
    assert!(shown.contains("Crew contract"), "{shown}");
    let hidden = text(
        &mut rig.world,
        rig.competitor,
        &format!("+sortie/contract #{}", npc.0),
    );
    let missing = text(
        &mut rig.world,
        rig.competitor,
        "+sortie/contract #4294967295",
    );
    assert_eq!(hidden, missing);
    let competing = submit(&mut rig.world, rig.competitor, &hire(npc, &terms()?)?)?;
    assert!(stage(&mut rig.world, &store, &competing).await.is_err());
    assert_eq!(contract(&rig)?, Some(agreement.clone()));
    let walked = text(&mut rig.world, npc, "crew-test-board");
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(rig.away),
        "ordinary NPC movement failed: {walked}"
    );
    assert_eq!(
        contract(&rig)?,
        Some(agreement),
        "walking does not update employment or create an embarkation interval"
    );
    Ok(())
}
#[tokio::test]
async fn changed_terms_or_npc_departure_refuses_hiring_through_final_admission() -> TestResult {
    for change_terms in [false, true] {
        let store = InMemoryStore::new();
        let mut rig = fixture(&store).await?;
        let request = submit(&mut rig.world, rig.employer, &hire(rig.npc, &terms()?)?)?;
        stage(&mut rig.world, &store, &request).await?;
        if change_terms {
            let mut changed = terms()?;
            changed.duties.insert(CrewDuty::Combat);
            rig.world.world_mut().set_attr(
                rig.npc,
                OFFER_ATTR,
                Attr::json(serde_json::to_value(changed)?, GOD),
            )?;
        } else {
            rig.world.world_mut().move_to(rig.npc, rig.away)?;
        }
        assert!(
            rig.world.engine_mut().take_staged_action().is_err(),
            "stale hiring facts reached the writer"
        );
        assert!(contract(&rig)?.is_none());
    }
    Ok(())
}
#[tokio::test]
async fn hiring_rejects_player_authored_consent_and_stale_displayed_terms() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    rig.world.world_mut().set_attr(
        rig.npc,
        OFFER_ATTR,
        Attr::json(serde_json::to_value(terms()?)?, rig.employer),
    )?;
    let request = submit(&mut rig.world, rig.employer, &hire(rig.npc, &terms()?)?)?;
    assert!(stage(&mut rig.world, &store, &request).await.is_err());
    let mut changed = terms()?;
    changed.mission_scope = NpcMissionScope::DetachmentCommander;
    rig.world.world_mut().set_attr(
        rig.npc,
        OFFER_ATTR,
        Attr::json(serde_json::to_value(changed)?, GOD),
    )?;
    assert!(
        stage(&mut rig.world, &store, &request).await.is_err(),
        "an old displayed hash cannot accept new duties"
    );
    assert!(contract(&rig)?.is_none());
    rig.world.world_mut().set_attr(
        rig.npc,
        OFFER_ATTR,
        Attr::json(serde_json::to_value(terms()?)?, GOD),
    )?;
    rig.world.world_mut().move_to(rig.npc, rig.away)?;
    rig.world.world_mut().move_to(rig.employer, rig.away)?;
    assert!(
        stage(&mut rig.world, &store, &request).await.is_err(),
        "an NPC's port label does not turn another room into that port"
    );
    Ok(())
}

#[tokio::test]
async fn preparation_resolves_hired_crew_from_current_hull_contents_and_keeps_contracts_private()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let employer = rig.employer;
    let npc = rig.npc;
    commit_line(&mut rig, &store, employer, &hire(npc, &terms()?)?).await?;
    let carrier =
        rig.world
            .world_mut()
            .create(ObjKind::Thing, "Crew carrier", GOD, Some(rig.room))?;
    for (object, behavior) in [(carrier, "sim.participant"), (rig.away, "sim.interior")] {
        rig.world.world_mut().attach_behavior(
            object,
            mush_core::BehaviorId::new(behavior)?,
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )?;
    }
    persist_legacy(&mut rig.world, &store).await?;
    let author_space = format!("@spatial/space #{}=moving:#{}", rig.away.0, carrier.0);
    commit_line(&mut rig, &store, GOD, &author_space).await?;
    let ashore =
        rig.world
            .world_mut()
            .create(ObjKind::Exit, "crew-test-ashore", GOD, Some(rig.away))?;
    rig.world.world_mut().link_exit(ashore, Some(rig.room))?;
    persist_legacy(&mut rig.world, &store).await?;
    let payload = serde_json::json!({"carrier":carrier.0,"slots":[
        {"slot":1,"unit":null,"controller":{"kind":"npc","character":npc.0},"role":null,"mind":null},
        {"slot":2,"unit":null,"controller":{"kind":"player","character":rig.competitor.0},"role":null,"mind":null}],
        "npc_commander":null,"financial_terms":null});
    commit_line(
        &mut rig,
        &store,
        employer,
        &format!("+sortie/draft {payload}"),
    )
    .await?;
    let from_port = text(&mut rig.world, employer, "+sortie/plan 1");
    assert!(
        from_port.contains("Board the selected carrier"),
        "{from_port}"
    );
    text(&mut rig.world, employer, "crew-test-board");
    let waiting = text(&mut rig.world, employer, "+sortie/plan 1");
    assert!(
        waiting.contains("not currently aboard the selected carrier"),
        "{waiting}"
    );
    text(&mut rig.world, npc, "crew-test-board");
    let aboard = text(&mut rig.world, employer, "+sortie/plan 1");
    assert!(aboard.contains("aboard now; contract recorded"), "{aboard}");
    assert!(aboard.contains("duties: mining, recovery"));
    assert!(!aboard.contains("last-port departure personnel proof"));
    text(&mut rig.world, rig.competitor, "crew-test-board");
    let invited = text(
        &mut rig.world,
        rig.competitor,
        &format!("+sortie/plan {}:1", employer.0),
    );
    assert!(
        !invited.contains("contract recorded"),
        "invitations do not disclose another employer's contracts"
    );
    assert!(!invited.contains("duties: mining"));
    let agreement = contract(&rig)?;
    text(&mut rig.world, npc, "crew-test-ashore");
    let left = text(&mut rig.world, employer, "+sortie/plan 1");
    assert!(
        left.contains("not currently aboard the selected carrier"),
        "{left}"
    );
    text(&mut rig.world, npc, "crew-test-board");
    let returned = text(&mut rig.world, employer, "+sortie/plan 1");
    assert!(
        returned.contains("aboard now; contract recorded"),
        "{returned}"
    );
    assert_eq!(contract(&rig)?, agreement);
    persist_legacy(&mut rig.world, &store).await?;
    drop(rig);
    let mut rig = fixture(&store).await?;
    let recovered = text(&mut rig.world, employer, "+sortie/plan 1");
    assert!(
        recovered.contains("aboard now; contract recorded"),
        "{recovered}"
    );
    Ok(())
}

async fn activation_fixture(store: &InMemoryStore) -> TestResult<(Rig, ObjId, ObjId)> {
    let mut rig = fixture(store).await?;
    let employer = rig.employer;
    let npc = rig.npc;
    commit_line(&mut rig, store, employer, &hire(npc, &terms()?)?).await?;
    let carrier =
        rig.world
            .world_mut()
            .create(ObjKind::Thing, "Activation carrier", GOD, Some(rig.room))?;
    let unit =
        rig.world
            .world_mut()
            .create(ObjKind::Thing, "Activation unit", GOD, Some(rig.away))?;
    for (object, behavior) in [
        (carrier, "sim.participant"),
        (unit, "sim.participant"),
        (rig.away, "sim.interior"),
    ] {
        rig.world.world_mut().attach_behavior(
            object,
            mush_core::BehaviorId::new(behavior)?,
            mush_core::BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )?;
    }
    let created = text(
        &mut rig.world,
        GOD,
        &format!("@unit/create #{}=tacsim.unit.line_tank", unit.0),
    );
    assert!(
        mush_tactics::module::unit_of(rig.world.world(), unit).is_some(),
        "{created}"
    );
    persist_legacy(&mut rig.world, store).await?;
    let bay = rig.away;
    commit_line(
        &mut rig,
        store,
        GOD,
        &format!("@spatial/space #{}=moving:#{}", bay.0, carrier.0),
    )
    .await?;
    text(
        &mut rig.world,
        GOD,
        &format!("@spatial/place/interior #{}=#{}", unit.0, bay.0),
    );
    text(
        &mut rig.world,
        GOD,
        &format!("@asset/title #{}=#{}", unit.0, employer.0),
    );
    let ashore =
        rig.world
            .world_mut()
            .create(ObjKind::Exit, "activation-ashore", GOD, Some(bay))?;
    rig.world.world_mut().link_exit(ashore, Some(rig.room))?;
    persist_legacy(&mut rig.world, store).await?;
    commit_line(
        &mut rig,
        store,
        employer,
        &format!(
            "@unitgrant/issue #{}=#{} operate_crew until=101",
            unit.0, npc.0
        ),
    )
    .await?;
    text(&mut rig.world, employer, "crew-test-board");
    text(&mut rig.world, npc, "crew-test-board");
    persist_legacy(&mut rig.world, store).await?;
    let plan = activation_plan(carrier, unit, npc);
    commit_line(&mut rig, store, employer, &format!("+sortie/draft {plan}")).await?;
    Ok((rig, carrier, unit))
}
fn activation_plan(carrier: ObjId, unit: ObjId, npc: ObjId) -> serde_json::Value {
    serde_json::json!({"carrier":carrier.0,"slots":[{"slot":1,"unit":unit.0,
        "controller":{"kind":"npc","character":npc.0},"role":null,"mind":null}],
        "npc_commander":null,"financial_terms":null})
}
fn active(
    rig: &Rig,
    id: u64,
) -> TestResult<Option<tacsim::detachment::activation::ActiveDetachment>> {
    let handles = rig
        .world
        .world()
        .module_state::<tacsim::detachment::activation::ActivationHandles>()
        .ok_or("activation handles")?;
    Ok(rig
        .world
        .engine()
        .action_state()
        .get(
            &handles.records,
            &tacsim::detachment::planning::DraftRef {
                organizer: rig.employer,
                id: mush_tactics::detachment::planning::PlanId(id),
            },
        )?
        .cloned())
}
fn reserved(rig: &Rig, entity: ObjId) -> TestResult<bool> {
    let handles = rig
        .world
        .world()
        .module_state::<tacsim::detachment::activation::ActivationHandles>()
        .ok_or("activation handles")?;
    Ok(rig
        .world
        .engine()
        .action_state()
        .get(&handles.reservations, &entity)?
        .is_some())
}
#[tokio::test]
async fn activation_is_atomic_recovers_lost_ack_and_releases_original_assignments() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, carrier, unit) = activation_fixture(&store).await?;
    let employer = rig.employer;
    let npc = rig.npc;
    let agreement = contract(&rig)?;
    let request = submit(&mut rig.world, employer, "+sortie/activate 1:1=mining")?;
    let ingress = stage(&mut rig.world, &store, &request).await?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("activation bundle")?
        .into_parts();
    assert_eq!(
        bundle.writes().len(),
        3,
        "only accepted roster and unit/person reservations"
    );
    assert!(active(&rig, 1)?.is_none());
    assert!(!reserved(&rig, unit)?);
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert!(active(&rig, 1)?.is_none());
    drop(installation);
    drop(rig);
    let mut rig = fixture(&store).await?;
    let accepted = active(&rig, 1)?.ok_or("recovered activation")?;
    assert!(accepted.active());
    assert!(reserved(&rig, unit)? && reserved(&rig, npc)?);
    assert_eq!(contract(&rig)?, agreement);
    let ActionPreflight::Finalized(retry) = ActionCoordinator::preflight(&store, ingress).await?
    else {
        return Err("activation retry not finalized".into());
    };
    assert_eq!(retry.outcome(), receipt.outcome());
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(rig.away)
    );
    let hidden = text(
        &mut rig.world,
        rig.competitor,
        &format!("+sortie/active {}:1", employer.0),
    );
    let missing = text(
        &mut rig.world,
        rig.competitor,
        &format!("+sortie/active {}:99", employer.0),
    );
    assert_eq!(
        hidden, missing,
        "other employers cannot probe accepted plans"
    );
    let plan = activation_plan(carrier, unit, npc);
    commit_line(&mut rig, &store, employer, &format!("+sortie/draft {plan}")).await?;
    let duplicate = submit(&mut rig.world, employer, "+sortie/activate 2:1=mining")?;
    assert!(stage(&mut rig.world, &store, &duplicate).await.is_err());
    assert!(active(&rig, 2)?.is_none());
    // Editing the draft neither releases nor mutates the accepted snapshot.
    let empty = serde_json::json!({"carrier":carrier.0,"slots":[],"npc_commander":null,"financial_terms":null});
    commit_line(
        &mut rig,
        &store,
        employer,
        &format!("+sortie/edit 1:1={empty}"),
    )
    .await?;
    assert_eq!(active(&rig, 1)?, Some(accepted));
    text(&mut rig.world, npc, "activation-ashore");
    assert!(
        reserved(&rig, npc)?,
        "walking is ordinary movement even while reserved"
    );
    text(
        &mut rig.world,
        GOD,
        &format!("@asset/transfer #{}=#{}", unit.0, rig.competitor.0),
    );
    persist_legacy(&mut rig.world, &store).await?;
    commit_line(&mut rig, &store, employer, "+sortie/deactivate 1:1").await?;
    assert!(!reserved(&rig, unit)? && !reserved(&rig, npc)?);
    assert!(!active(&rig, 1)?.ok_or("closed activation")?.active());
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(rig.room)
    );
    assert_eq!(contract(&rig)?, agreement);
    // A fresh activation has its own generation; stale release commands must
    // not release the replacement roster even when its assignments are equal.
    text(&mut rig.world, npc, "crew-test-board");
    text(
        &mut rig.world,
        GOD,
        &format!("@asset/transfer #{}=#{}", unit.0, employer.0),
    );
    persist_legacy(&mut rig.world, &store).await?;
    commit_line(
        &mut rig,
        &store,
        employer,
        &format!("@unitgrant/issue #{}=#{} operate_crew", unit.0, npc.0),
    )
    .await?;
    commit_line(
        &mut rig,
        &store,
        employer,
        &format!("+sortie/edit 1:2={plan}"),
    )
    .await?;
    commit_line(&mut rig, &store, employer, "+sortie/activate 1:3=mining").await?;
    let replacement = active(&rig, 1)?.ok_or("replacement activation")?;
    assert_eq!(replacement.revision, 2);
    assert_eq!(replacement.source_revision, 3);
    let stale = submit(&mut rig.world, employer, "+sortie/deactivate 1:1")?;
    assert!(stage(&mut rig.world, &store, &stale).await.is_err());
    assert!(reserved(&rig, unit)? && reserved(&rig, npc)?);
    commit_line(&mut rig, &store, employer, "+sortie/deactivate 1:2").await?;
    Ok(())
}

#[tokio::test]
async fn activation_rechecks_current_presence_and_unit_rights_at_final_take() -> TestResult {
    for change in ["departure", "returned", "expiry", "duty"] {
        let store = InMemoryStore::new();
        let (mut rig, _, unit) = activation_fixture(&store).await?;
        let employer = rig.employer;
        let command = if change == "duty" {
            "+sortie/activate 1:1=combat"
        } else {
            "+sortie/activate 1:1=mining"
        };
        let request = submit(&mut rig.world, employer, command)?;
        if change == "duty" {
            assert!(stage(&mut rig.world, &store, &request).await.is_err());
            continue;
        }
        stage(&mut rig.world, &store, &request).await?;
        if change == "expiry" {
            rig.world.world_mut().set_now(102);
        } else {
            text(&mut rig.world, rig.npc, "activation-ashore");
            if change == "returned" {
                text(&mut rig.world, rig.npc, "crew-test-board");
            }
        }
        let taken = rig.world.engine_mut().take_staged_action();
        if change == "returned" {
            assert!(
                taken?.is_some(),
                "current containment qualifies without an interval or movement history"
            );
        } else {
            assert!(taken.is_err(), "stale {change} reached the writer");
        }
        assert!(active(&rig, 1)?.is_none());
        assert!(!reserved(&rig, unit)?);
    }
    Ok(())
}

#[tokio::test]
async fn activation_cannot_use_an_offer_resolved_before_real_unit_grant_revocation() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _, unit) = activation_fixture(&store).await?;
    let employer = rig.employer;
    let request = submit(&mut rig.world, employer, "+sortie/activate 1:1=mining")?;
    let ingress = ActionCoordinator::command_ingress(&request, 1)?;
    let ActionPreflight::Vacant(permit) =
        ActionCoordinator::preflight(&store, ingress.clone()).await?
    else {
        return Err("activation not vacant".into());
    };
    let principal = mush_core::PrincipalId::Object { id: employer };
    let query = rig.world.engine().legal_action_query(
        principal.clone(),
        employer,
        request.subject.clone(),
    )?;
    let offers = rig.world.engine().project_legal_actions(&query)?;
    let projected = offers
        .projected_for_action(&request.action)
        .ok_or("activation offer")?;
    let resolved = rig.world.engine().resolve_legal_action(
        &offers,
        mush_core::OfferExecutionContext {
            request: ingress.request(),
            principal,
            actor: employer,
            idempotency: ingress.idempotency().clone(),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: request.targets.clone(),
            parameters: request.parameters.clone(),
        },
    )?;
    commit_line(&mut rig, &store, employer, "@unitgrant/revoke 1").await?;
    if ActionCoordinator::prepare_and_stage_resolved(permit, rig.world.engine_mut(), resolved)
        .is_ok()
    {
        assert!(
            rig.world.engine_mut().take_staged_action().is_err(),
            "a revoked right reached the writer"
        );
    }
    assert!(active(&rig, 1)?.is_none());
    assert!(!reserved(&rig, unit)?);
    Ok(())
}

async fn berthed_crew_carrier(
    rig: &mut Rig,
    store: &InMemoryStore,
) -> TestResult<(ObjId, ObjId, ObjId)> {
    let (hull, domain, gallery, crew) = {
        let port =
            tacsim::port::coop::programme_port(rig.world.world()).ok_or("real programme port")?;
        assert_eq!(port.def.venue.as_str(), "ceres");
        let row = port.map.hulls.first().ok_or("real programme hull")?;
        (
            ObjId(row.hull),
            port.map.domain.ok_or("port domain")?,
            ObjId(port.map.presentation.ok_or("berth gallery")?),
            ObjId(
                *row.rooms
                    .get(&CanonicalId::new("crew")?)
                    .ok_or("crew compartment")?,
            ),
        )
    };
    let transferred = text(
        &mut rig.world,
        GOD,
        &format!("@asset/transfer #{}=#{}", hull.0, rig.employer.0),
    );
    assert!(transferred.contains("Title transferred"), "{transferred}");
    let docked = text(
        &mut rig.world,
        GOD,
        &format!("@spatial/place/domain #{}={domain}", hull.0),
    );
    assert!(
        matches!(tacsim::port::ship::berth_of(rig.world.world(),hull),tacsim::port::ship::Berth::Docked {def,..} if def.venue.as_str()=="ceres"),
        "{docked}"
    );
    assert_eq!(
        tacsim::port::coop::helm_of(rig.world.world(), rig.employer)
            .ok_or("real captain")?
            .hull,
        hull
    );
    persist_legacy(&mut rig.world, store).await?;
    Ok((hull, gallery, crew))
}

#[tokio::test]
async fn hired_crew_board_through_the_passenger_path_and_retain_current_presence_after_restart()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let employer = rig.employer;
    let npc = rig.npc;
    commit_line(&mut rig, &store, employer, &hire(npc, &terms()?)?).await?;
    let agreement = contract(&rig)?;
    let (hull, gallery, crew) = berthed_crew_carrier(&mut rig, &store).await?;
    let boarded = text(
        &mut rig.world,
        employer,
        "+sortie/board Crew mining operator",
    );
    assert!(boarded.contains("has come aboard"), "{boarded}");
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(crew)
    );
    assert_eq!(
        rig.world.world().get(employer).and_then(|o| o.location),
        Some(rig.room),
        "arranging boarding does not move the captain"
    );
    assert_eq!(
        contract(&rig)?,
        agreement,
        "boarding changes no contract or aboard ledger"
    );
    assert!(rig.world.engine_mut().take_authoritative_submit().is_none());
    // The captain's existing boarding command uses the remaining real berth.
    rig.world.world_mut().move_to(employer, gallery)?;
    let captain = text(&mut rig.world, employer, "+ship/board");
    assert!(captain.contains("You cycle the lock into"), "{captain}");
    let placement = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("spatial state")?
        .placements
        .get(&npc)
        .cloned();
    let repeated = text(
        &mut rig.world,
        employer,
        &format!("+sortie/board #{npc_id}", npc_id = npc.0),
    );
    assert!(repeated.contains("already aboard"), "{repeated}");
    assert_eq!(
        rig.world
            .world()
            .module_state::<mush_sim::SpatialState>()
            .ok_or("spatial state")?
            .placements
            .get(&npc),
        placement.as_ref()
    );
    persist_legacy(&mut rig.world, &store).await?;
    let payload = serde_json::json!({"carrier":hull.0,"slots":[{"slot":1,"unit":null,"controller":{"kind":"npc","character":npc.0},"role":null,"mind":null}],"npc_commander":null,"financial_terms":null});
    commit_line(
        &mut rig,
        &store,
        employer,
        &format!("+sortie/draft {payload}"),
    )
    .await?;
    let prepared = text(&mut rig.world, employer, "+sortie/plan 1");
    assert!(
        prepared.contains("aboard now; contract recorded"),
        "{prepared}"
    );
    drop(rig);
    let mut rig = fixture(&store).await?;
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(crew)
    );
    assert_eq!(contract(&rig)?, agreement);
    let prepared = text(&mut rig.world, employer, "+sortie/plan 1");
    assert!(
        prepared.contains("aboard now; contract recorded"),
        "{prepared}"
    );
    let repeated = text(
        &mut rig.world,
        employer,
        &format!("+sortie/board #{}", npc.0),
    );
    assert!(repeated.contains("already aboard"), "{repeated}");
    Ok(())
}

#[tokio::test]
async fn crew_boarding_requires_own_contract_and_present_captain_and_obeys_station_exit_locks()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let employer = rig.employer;
    let npc = rig.npc;
    let _ = berthed_crew_carrier(&mut rig, &store).await?;
    let command = format!("+sortie/board #{}", npc.0);
    let missing = text(&mut rig.world, employer, &command);
    assert!(missing.contains("contract unavailable"), "{missing}");
    commit_line(&mut rig, &store, employer, &hire(npc, &terms()?)?).await?;
    let other = text(&mut rig.world, rig.competitor, &command);
    let absent = text(&mut rig.world, rig.competitor, "+sortie/board #4294967295");
    assert_eq!(other, absent);
    rig.world.world_mut().move_to(employer, rig.away)?;
    let away = text(&mut rig.world, employer, &command);
    assert!(away.contains("from this port or aboard"), "{away}");
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(rig.room)
    );
    rig.world.world_mut().move_to(employer, rig.room)?;
    let exits = rig.world.world().exits_of(rig.room).to_vec();
    for exit in &exits {
        text(&mut rig.world, GOD, &format!("@lock #{}=#FALSE", exit.0));
    }
    let locked = text(&mut rig.world, employer, &command);
    assert!(locked.contains("stopped on the way"), "{locked}");
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(rig.room)
    );
    for exit in &exits {
        text(&mut rig.world, GOD, &format!("@lock #{}=#TRUE", exit.0));
    }
    let retried = text(&mut rig.world, employer, &command);
    assert!(retried.contains("has come aboard"), "{retried}");
    Ok(())
}

#[tokio::test]
async fn crew_boarding_from_aboard_obeys_real_berth_capacity_and_retries_from_the_gallery()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let employer = rig.employer;
    let npc = rig.npc;
    commit_line(&mut rig, &store, employer, &hire(npc, &terms()?)?).await?;
    let agreement = contract(&rig)?;
    let (_, gallery, crew) = berthed_crew_carrier(&mut rig, &store).await?;
    rig.world.world_mut().move_to(employer, gallery)?;
    let captain = text(&mut rig.world, employer, "+ship/board");
    assert!(captain.contains("You cycle the lock into"), "{captain}");
    let mush_sim::SpatialParent::Interior { space, slot } = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("spatial state")?
        .parent_of(employer)
    else {
        return Err("captain's berth".into());
    };
    let slot = slot.ok_or("actual berth slot")?;
    let guest =
        rig.world
            .world_mut()
            .create(ObjKind::Npc, "Existing crew guest", GOD, Some(space))?;
    rig.world.world_mut().attach_behavior(
        guest,
        mush_core::BehaviorId::new("sim.participant")?,
        mush_core::BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 0,
        },
    )?;
    let seated = text(
        &mut rig.world,
        GOD,
        &format!("@spatial/place/interior #{}=#{},{}", guest.0, space.0, slot),
    );
    assert!(
        matches!(rig.world.world().module_state::<mush_sim::SpatialState>().ok_or("spatial state")?.parent_of(guest),mush_sim::SpatialParent::Interior {slot:Some(ref used),..} if used==&slot),
        "{seated}"
    );
    let command = format!("+sortie/board #{}", npc.0);
    let full = text(&mut rig.world, employer, &command);
    assert!(full.contains("no available passenger berth"), "{full}");
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(gallery),
        "the real walk to the berth is retained"
    );
    assert_eq!(contract(&rig)?, agreement);
    text(
        &mut rig.world,
        GOD,
        &format!("@spatial/unplace #{}", guest.0),
    );
    text(
        &mut rig.world,
        GOD,
        &format!("@teleport #{}=#{}", guest.0, gallery.0),
    );
    let retried = text(&mut rig.world, employer, &command);
    assert!(retried.contains("has come aboard"), "{retried}");
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(crew)
    );
    assert_eq!(contract(&rig)?, agreement);
    Ok(())
}

#[tokio::test]
async fn launch_inspection_reads_accepted_assignments_and_current_presence_without_writing()
-> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, carrier, unit) = activation_fixture(&store).await?;
    let employer = rig.employer;
    let npc = rig.npc;
    commit_line(&mut rig, &store, employer, "+sortie/activate 1:1=mining").await?;
    let accepted = active(&rig, 1)?.ok_or("accepted roster")?;
    let agreement = contract(&rig)?;
    // An edited draft cannot redirect this inspection to another unit/person.
    let changed = activation_plan(carrier, unit, rig.competitor);
    commit_line(
        &mut rig,
        &store,
        employer,
        &format!("+sortie/edit 1:1={changed}"),
    )
    .await?;
    let inspection = text(&mut rig.world, employer, "+sortie/preflight 1:1");
    assert!(
        inspection.contains("accepted draft revision 1"),
        "{inspection}"
    );
    assert!(
        inspection.contains(&format!("operator #{}.", npc.0)),
        "{inspection}"
    );
    assert!(
        inspection.contains("reservations intact: yes"),
        "{inspection}"
    );
    assert!(
        inspection.contains("Operator OperateCrew authority: allowed now"),
        "{inspection}"
    );
    assert!(
        inspection.contains("accepted contract and duty still match"),
        "{inspection}"
    );
    assert!(inspection.contains("Unit condition:"), "{inspection}");
    assert!(
        inspection.contains("Carrier surface berth unavailable"),
        "{inspection}"
    );
    assert!(inspection.contains("Observations only"), "{inspection}");
    assert!(rig.world.engine_mut().take_authoritative_submit().is_none());
    assert!(!rig.world.engine().has_legacy_persistence_work());
    text(&mut rig.world, npc, "activation-ashore");
    let left = text(&mut rig.world, employer, "+sortie/preflight 1:1");
    assert!(
        left.contains("not currently aboard the selected carrier"),
        "{left}"
    );
    text(&mut rig.world, npc, "crew-test-board");
    let returned = text(&mut rig.world, employer, "+sortie/preflight 1:1");
    assert!(
        returned.contains("accepted contract and duty still match"),
        "{returned}"
    );
    assert_eq!(active(&rig, 1)?.as_ref(), Some(&accepted));
    assert_eq!(contract(&rig)?, agreement);
    assert!(reserved(&rig, unit)?);
    assert!(reserved(&rig, npc)?);
    let damage = serde_json::json!({"ops":[{"SetComponentCondition":{"component":"power","condition":"destroyed"}}]});
    let damaged = text(
        &mut rig.world,
        GOD,
        &format!("@unit/damage #{}={damage}", unit.0),
    );
    let after_damage = text(&mut rig.world, employer, "+sortie/preflight 1:1");
    assert!(
        after_damage.contains("power no; control no"),
        "{damaged}\n{after_damage}"
    );
    persist_legacy(&mut rig.world, &store).await?;
    drop(rig);
    let mut rig = fixture(&store).await?;
    let rebooted = text(&mut rig.world, employer, "+sortie/preflight 1:1");
    assert!(
        rebooted.contains("accepted contract and duty still match"),
        "{rebooted}"
    );
    let stale = text(&mut rig.world, employer, "+sortie/preflight 1:2");
    assert!(stale.contains("Accepted roster unavailable"), "{stale}");
    let other = text(&mut rig.world, rig.competitor, "+sortie/preflight 1:1");
    assert!(!other.contains("NPC personnel"), "{other}");
    text(&mut rig.world, employer, "activation-ashore");
    let ashore = text(&mut rig.world, employer, "+sortie/preflight 1:1");
    assert!(ashore.contains("Board the selected carrier"), "{ashore}");
    assert!(!ashore.contains("NPC personnel"));
    Ok(())
}

#[tokio::test]
async fn launch_inspection_rechecks_real_crew_grants_and_withholds_details_after_deploy_loss()
-> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _, unit) = activation_fixture(&store).await?;
    let employer = rig.employer;
    commit_line(&mut rig, &store, employer, "+sortie/activate 1:1=mining").await?;
    commit_line(&mut rig, &store, employer, "@unitgrant/revoke 1").await?;
    let revoked = text(&mut rig.world, employer, "+sortie/preflight 1:1");
    assert!(
        revoked.contains("Your Deploy authority: allowed now"),
        "{revoked}"
    );
    assert!(
        revoked.contains("Operator OperateCrew authority: denied now"),
        "{revoked}"
    );
    assert!(
        revoked.contains("accepted contract and duty still match"),
        "{revoked}"
    );
    let transferred = text(
        &mut rig.world,
        GOD,
        &format!("@asset/transfer #{}=#{}", unit.0, rig.competitor.0),
    );
    assert!(transferred.contains("Title transferred"), "{transferred}");
    let denied = text(&mut rig.world, employer, "+sortie/preflight 1:1");
    assert!(
        denied.contains("Your Deploy authority: denied now"),
        "{denied}"
    );
    assert!(!denied.contains("Unit condition:"), "{denied}");
    assert!(
        !denied.contains("Operator OperateCrew authority:"),
        "{denied}"
    );
    assert!(reserved(&rig, unit)?);
    assert!(active(&rig, 1)?.is_some_and(|r| r.active()));
    Ok(())
}

#[tokio::test]
async fn shipped_port_crew_can_be_hired_without_staff_offers_and_keep_identity_after_boot()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let person = |world: &TestWorld| -> TestResult<ObjId> {
        world
            .world()
            .module_state::<mush_content::ContentState>()
            .and_then(|s| {
                s.latches
                    .applied(&CanonicalId::new(tacsim::detachment::crew::content::PACK).ok()?)
            })
            .and_then(|l| l.entries().get(&CanonicalId::new("crew.ilya_mendes").ok()?))
            .and_then(mush_content::AppliedEntry::object)
            .ok_or_else(|| "authored crew latch".into())
    };
    rig.npc = person(&rig.world)?;
    let npc = rig.npc;
    let employer = rig.employer;
    let offer = rig
        .world
        .world()
        .attr(npc, OFFER_ATTR)
        .ok_or("authored consent")?;
    assert_eq!(offer.owner, GOD);
    assert!(offer.flags.contains(
        mush_core::AttrFlags::WIZARD_ONLY
            | mush_core::AttrFlags::HIDDEN
            | mush_core::AttrFlags::NO_INHERIT
    ));
    let terms: HireTerms = serde_json::from_value(offer.as_json().ok_or("terms")?.clone())?;
    let listed = text(&mut rig.world, employer, "+sortie/crew");
    assert!(
        listed.contains("Ilya Mendes") && listed.contains("Sana Okonkwo"),
        "{listed}"
    );
    assert!(listed.contains(&hire(npc, &terms)?));
    commit_line(&mut rig, &store, employer, &hire(npc, &terms)?).await?;
    let saved = contract(&rig)?.ok_or("actual accepted contract")?;
    // Ordinary movement changes current contents; boot must not restock a crew.
    text(&mut rig.world, npc, "crew-test-board");
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(rig.away)
    );
    rig.world.world_mut().set_attr(
        npc,
        mush_npc::SUMMARY_ATTR,
        Attr::text("My first mining contract.", GOD),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    let away = rig.away;
    drop(rig);
    let mut rig = fixture(&store).await?;
    assert_eq!(person(&rig.world)?, npc);
    rig.npc = npc;
    assert_eq!(contract(&rig)?.as_ref(), Some(&saved));
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(away)
    );
    assert_eq!(
        rig.world
            .world()
            .attr(npc, mush_npc::SUMMARY_ATTR)
            .and_then(Attr::as_text),
        Some("My first mining contract.")
    );
    assert_eq!(
        rig.world
            .world()
            .iter()
            .filter(|o| o.name == "Ilya Mendes")
            .count(),
        1
    );
    let listed = text(&mut rig.world, employer, "+sortie/crew");
    assert!(
        !listed.contains("Ilya Mendes"),
        "a hired absent person is unavailable: {listed}"
    );
    assert!(listed.contains("Sana Okonkwo"));
    Ok(())
}
