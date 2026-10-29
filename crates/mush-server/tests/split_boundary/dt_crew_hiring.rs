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
