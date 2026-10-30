//! Private plans use the actual production roster and acknowledged coordinator.
use std::sync::Arc;

use mush_core::{
    AccountId, AccountIdentity, BehaviorBinding, BehaviorId, Engine, GOD, ObjId, ObjKind,
    OfferKeyRing, OfferSecret, ScheduleDriveCause,
};
use mush_server::action_coordinator::{ActionCoordinator, ActionPreflight};
use mush_store::{AuthoritativeStore, InMemoryStore};
use mush_tactics::detachment::planning::{ConsentState, Draft, PlanId, PlanningBook};
use mush_testkit::{TestWorld, text_of};
use serde_json::{Value, json};
use tacsim::detachment::planning::PlanningHandles;

use super::dt_unit_authority::{persist_legacy, stage, submit};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct Rig {
    world: TestWorld,
    organizer: ObjId,
    invitee: ObjId,
    stranger: ObjId,
    carrier: ObjId,
    unit: ObjId,
}

async fn fixture(store: &InMemoryStore) -> TestResult<Rig> {
    let mut config = mush_config::config::Config::default();
    config.tacsim.enabled = true;
    let llm = Arc::new(mush_llm::LlmService::from_config(
        &mush_llm::LlmConfig::default(),
    )?);
    let modules = mush_server::boot::game_modules(&config, &llm)?;
    let mut world = mush_server::persist::load_or_seed_world(store).await?;
    let mut ensure = |name: &str, kind| -> TestResult<ObjId> {
        let existing = world.iter().find(|row| row.name == name).map(|row| row.id);
        Ok(match existing {
            Some(id) => id,
            None => world.create(kind, name, GOD, Some(mush_core::LIMBO))?,
        })
    };
    let organizer = ensure("Planning organizer", ObjKind::Player)?;
    let invitee = ensure("Planning invitee", ObjKind::Player)?;
    let stranger = ensure("Planning outsider", ObjKind::Player)?;
    let carrier = ensure("Planning carrier", ObjKind::Thing)?;
    let unit = ensure("Hidden preferred unit name", ObjKind::Thing)?;
    // Arm real periodic ticks on the same deterministic wall clock used below.
    world.set_now(100);
    let mut engine =
        Engine::with_modules(world, mush_server::world_task::base_registry(), modules)?;
    mush_server::modules::boot_modules(&mut engine, store).await?;
    engine.world.set_now(100);
    engine.set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x42; 32])?, [])?);
    let mut world = TestWorld::from_engine(engine);
    persist_legacy(&mut world, store).await?;
    Ok(Rig {
        world,
        organizer,
        invitee,
        stranger,
        carrier,
        unit,
    })
}

fn text(world: &mut TestWorld, actor: ObjId, command: &str) -> String {
    world
        .run(actor, command)
        .iter()
        .map(|(_, event)| text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

fn plan(rig: &Rig) -> Value {
    json!({"carrier":rig.carrier.0,"slots":[{"slot":1,"unit":rig.unit.0,
        "controller":{"kind":"player","character":rig.invitee.0},"role":"scout","mind":null}],
        "npc_commander":null,"financial_terms":null})
}

fn book(world: &TestWorld, owner: ObjId) -> TestResult<PlanningBook> {
    let handles = world
        .world()
        .module_state::<PlanningHandles>()
        .ok_or("planning handles")?;
    world
        .engine()
        .action_state()
        .get(&handles.families.books, &owner)?
        .cloned()
        .ok_or_else(|| "planning book missing".into())
}

fn draft(world: &TestWorld, owner: ObjId, id: u64) -> TestResult<Draft> {
    book(world, owner)?
        .drafts
        .get(&PlanId(id))
        .cloned()
        .ok_or_else(|| "draft missing".into())
}

fn declaration(verb: &str, draft: &Draft) -> String {
    format!(
        "+sortie/{verb} {}:{}:{}:{}",
        draft.organizer.0,
        draft.id.0,
        draft.revision,
        draft.plan_hash.as_str()
    )
}

async fn commit(
    world: &mut TestWorld,
    store: &InMemoryStore,
    actor: ObjId,
    line: &str,
) -> TestResult {
    let submission = submit(world, actor, line).map_err(|error| format!("{line}: {error}"))?;
    stage(world, store, &submission)
        .await
        .map_err(|error| format!("{line}: {error}"))?;
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged plan")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    world.engine_mut().install_composed_action(installation)?;
    Ok(())
}

#[tokio::test]
async fn presets_drafts_and_actor_consent_survive_lost_ack_without_becoming_authority() -> TestResult
{
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let payload = plan(&rig);
    let organizer = rig.organizer;
    let invitee = rig.invitee;
    let stranger = rig.stranger;
    commit(
        &mut rig.world,
        &store,
        organizer,
        &format!("+sortie/preset Private patrol={payload}"),
    )
    .await?;
    commit(&mut rig.world, &store, organizer, "+sortie/load 1:1").await?;
    commit(&mut rig.world, &store, organizer, "+sortie/load 1:1").await?;
    let original = book(&rig.world, organizer)?;
    assert_eq!(original.presets.len(), 1);
    assert_eq!(
        original.drafts.len(),
        2,
        "independent plans may prefer the same unit"
    );
    let accepted = draft(&rig.world, organizer, 2)?;
    assert!(
        text(
            &mut rig.world,
            stranger,
            &format!("+sortie/plan {}:2", organizer.0)
        )
        .contains("unavailable")
    );
    assert!(text(&mut rig.world, stranger, "+sortie/plans").contains("No saved plans"));
    let invited = text(&mut rig.world, invitee, "+sortie/invitations");
    assert!(invited.contains(&format!("+sortie/plan {}:2", organizer.0)));
    assert!(
        !invited.contains("Private patrol"),
        "invitation does not reveal private preset names"
    );
    let view = text(
        &mut rig.world,
        invitee,
        &format!("+sortie/plan {}:2", organizer.0),
    );
    assert!(view.contains(&declaration("accept", &accepted)));
    assert!(
        !view.contains("Hidden preferred unit name"),
        "unresolved references disclose no object name"
    );
    assert!(
        submit(&mut rig.world, organizer, &declaration("accept", &accepted)).is_err(),
        "organizer cannot consent for another actor"
    );
    let request = submit(&mut rig.world, invitee, &declaration("accept", &accepted))?;
    let ingress = stage(&mut rig.world, &store, &request).await?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged acceptance")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert!(
        draft(&rig.world, organizer, 2)?.consents.is_empty(),
        "missing acknowledgement publishes nothing"
    );
    drop(installation);
    drop(rig);
    let mut rig = fixture(&store).await?;
    let accepted = draft(&rig.world, organizer, 2)?;
    assert_eq!(
        accepted
            .consents
            .get(&invitee)
            .ok_or("restarted consent")?
            .state,
        ConsentState::Accepted
    );
    assert!(draft(&rig.world, organizer, 3)?.consents.is_empty());
    let ActionPreflight::Finalized(retry) = ActionCoordinator::preflight(&store, ingress).await?
    else {
        return Err("acceptance retry lost its durable receipt".into());
    };
    assert_eq!(retry.token(), bundle.token());
    assert_eq!(retry.outcome(), receipt.outcome());
    assert!(
        submit(&mut rig.world, invitee, &declaration("ready", &accepted)).is_err(),
        "acceptance is not physical seating"
    );
    commit(
        &mut rig.world,
        &store,
        invitee,
        &declaration("decline", &accepted),
    )
    .await?;
    assert!(
        !text(&mut rig.world, invitee, "+sortie/invitations")
            .contains(&format!("+sortie/plan {}:2", organizer.0)),
        "declining frees invitation capacity"
    );
    commit(
        &mut rig.world,
        &store,
        invitee,
        &declaration("accept", &accepted),
    )
    .await?;
    assert!(
        text(&mut rig.world, invitee, "+sortie/invitations")
            .contains(&format!("+sortie/plan {}:2", organizer.0)),
        "explicit acceptance restores the invitation"
    );
    let mut changed = payload;
    changed["slots"][0]["role"] = json!("support");
    commit(
        &mut rig.world,
        &store,
        organizer,
        &format!("+sortie/edit 2:1={changed}"),
    )
    .await?;
    let edited = draft(&rig.world, organizer, 2)?;
    assert_eq!(edited.revision, 2);
    assert!(edited.consents.is_empty());
    assert_ne!(edited.plan_hash, accepted.plan_hash);
    assert!(submit(&mut rig.world, invitee, &declaration("accept", &accepted)).is_err());
    assert_eq!(
        book(&rig.world, organizer)?.presets,
        original.presets,
        "draft edits cannot rewrite the preset"
    );
    assert_eq!(
        draft(&rig.world, organizer, 3)?.plan,
        original.drafts.get(&PlanId(3)).ok_or("second draft")?.plan
    );
    commit(&mut rig.world, &store, organizer, "+sortie/discard 2:2").await?;
    assert!(
        !text(&mut rig.world, invitee, "+sortie/invitations")
            .contains(&format!("+sortie/plan {}:2", organizer.0))
    );
    Ok(())
}

#[tokio::test]
async fn replacement_input_cannot_reinterpret_an_earlier_queued_plan_selector() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let payload = plan(&rig);
    let old = submit(
        &mut rig.world,
        rig.organizer,
        &format!("+sortie/preset Old={payload}"),
    )?;
    let newer = submit(
        &mut rig.world,
        rig.organizer,
        &format!("+sortie/preset New={payload}"),
    )?;
    assert!(stage(&mut rig.world, &store, &old).await.is_err());
    stage(&mut rig.world, &store, &newer).await?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("new selector")?
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
    let book = book(&rig.world, rig.organizer)?;
    assert_eq!(book.presets.len(), 1);
    assert_eq!(book.presets.get(&PlanId(1)).ok_or("preset")?.name, "New");
    Ok(())
}

struct PilotAccount {
    actor: ObjId,
    account: AccountId,
}
impl AccountIdentity for PilotAccount {
    fn account_of(&self, actor: ObjId) -> Option<AccountId> {
        (actor == self.actor).then_some(self.account)
    }
}

fn ordinary(world: &mut TestWorld, actor: ObjId, command: &str) -> TestResult {
    let events = world.run(actor, command);
    if events.iter().any(|(_, event)| {
        matches!(event, mush_proto::OutputEvent::Sys(text)
        if text.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
    }) {
        return Err(format!(
            "{command}: {:?}",
            events
                .iter()
                .map(|(_, event)| text_of(event))
                .collect::<Vec<_>>()
        )
        .into());
    }
    Ok(())
}

async fn install_due_handoff(world: &mut TestWorld, store: &InMemoryStore) -> TestResult {
    let page = world
        .engine()
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 64)?;
    let candidate = page
        .candidates()
        .iter()
        .find(|candidate| candidate.action().key().id.as_str() == "control_handoff_apply")
        .ok_or("due real handoff")?
        .clone();
    let ingress = ActionCoordinator::scheduled_ingress(candidate.action(), candidate.advance(), 1)?;
    let ActionPreflight::Vacant(permit) = ActionCoordinator::preflight(store, ingress).await?
    else {
        return Err("handoff schedule was not vacant".into());
    };
    ActionCoordinator::prepare_and_stage_scheduled(permit, world.engine_mut(), &candidate)?;
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged handoff")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    world.engine_mut().install_composed_action(installation)?;
    Ok(())
}

async fn seated_fixture(rig: &mut Rig, store: &InMemoryStore) -> TestResult<ObjId> {
    let field = rig
        .world
        .world_mut()
        .create(ObjKind::Room, "Preparation field", GOD, None)?;
    let bay = rig
        .world
        .world_mut()
        .create(ObjKind::Room, "Preparation bay", GOD, None)?;
    for (object, behavior) in [
        (field, "sim.domain"),
        (bay, "sim.interior"),
        (rig.carrier, "sim.participant"),
        (rig.unit, "sim.participant"),
    ] {
        rig.world.world_mut().attach_behavior(
            object,
            BehaviorId::new(behavior)?,
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )?;
    }
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@unit/create #{}=tacsim.unit.line_tank", rig.unit.0),
    )?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@domain/create #{}=sim.passive", field.0),
    )?;
    let domain = rig
        .world
        .world()
        .module_state::<mush_sim::data::SpatialState>()
        .ok_or("spatial state")?
        .domains
        .values()
        .find(|domain| domain.presentation_location == field)
        .ok_or("created domain")?
        .id;
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/place/domain #{}={}", rig.carrier.0, domain.0),
    )?;
    persist_legacy(&mut rig.world, store).await?;
    commit(
        &mut rig.world,
        store,
        GOD,
        &format!("@spatial/space #{}=moving:#{}", bay.0, rig.carrier.0),
    )
    .await?;
    for command in [
        format!("@spatial/place/interior #{}=#{}", rig.unit.0, bay.0),
        format!(
            "@spatial/station #{}=pilot,pilot,channels:helm,space:#{}",
            rig.unit.0, bay.0
        ),
        format!("@spatial/assign #{}=pilot,#{}", rig.unit.0, rig.invitee.0),
    ] {
        ordinary(&mut rig.world, GOD, &command)?;
    }
    rig.world.world_mut().move_to(rig.invitee, bay)?;
    *rig.world
        .world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .ok_or("account oracle")? = mush_sim::AccountOracle::new(Arc::new(PilotAccount {
        actor: rig.invitee,
        account: AccountId::mint("PlanningPilot", 1)?,
    }));
    ordinary(
        &mut rig.world,
        rig.invitee,
        &format!("+focus #{}/pilot", rig.unit.0),
    )?;
    Ok(bay)
}

#[tokio::test]
async fn ready_requires_real_helm_lease_and_final_physical_revalidation() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let bay = seated_fixture(&mut rig, &store).await?;
    persist_legacy(&mut rig.world, &store).await?;
    let payload = plan(&rig);
    commit(
        &mut rig.world,
        &store,
        rig.organizer,
        &format!("+sortie/draft {payload}"),
    )
    .await?;
    let selected = draft(&rig.world, rig.organizer, 1)?;
    commit(
        &mut rig.world,
        &store,
        rig.invitee,
        &declaration("accept", &selected),
    )
    .await?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@asset/title #{}=#{}", rig.unit.0, GOD.0),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    commit(
        &mut rig.world,
        &store,
        GOD,
        &format!(
            "@unitgrant/issue #{}=#{} operate_crew",
            rig.unit.0, rig.invitee.0
        ),
    )
    .await?;
    let report = text(
        &mut rig.world,
        rig.invitee,
        &format!("+sortie/plan {}:1", rig.organizer.0),
    );
    assert!(
        report.contains("allowed now"),
        "Tacsim uses its own registered Org reader: {report}"
    );
    commit(&mut rig.world, &store, GOD, "@unitgrant/revoke 1").await?;
    let report = text(
        &mut rig.world,
        rig.invitee,
        &format!("+sortie/plan {}:1", rig.organizer.0),
    );
    assert!(
        report.contains("denied now"),
        "reports read current grant revocation: {report}"
    );
    let request = submit(
        &mut rig.world,
        rig.invitee,
        &declaration("ready", &selected),
    )?;
    assert!(
        stage(&mut rig.world, &store, &request).await.is_err(),
        "billet and focus alone are not the real typed helm lease"
    );
    commit(
        &mut rig.world,
        &store,
        rig.invitee,
        &format!("@station/handoff #{}/pilot", rig.unit.0),
    )
    .await?;
    install_due_handoff(&mut rig.world, &store).await?;
    let request = submit(
        &mut rig.world,
        rig.invitee,
        &declaration("ready", &selected),
    )?;
    stage(&mut rig.world, &store, &request).await?;
    rig.world
        .world_mut()
        .move_to(rig.invitee, mush_core::LIMBO)?;
    assert!(
        rig.world.engine_mut().take_staged_action().is_err(),
        "leaving the physical station invalidates final take"
    );
    assert_eq!(
        draft(&rig.world, rig.organizer, 1)?
            .consents
            .get(&rig.invitee)
            .ok_or("accepted")?
            .state,
        ConsentState::Accepted
    );
    rig.world.world_mut().move_to(rig.invitee, bay)?;
    persist_legacy(&mut rig.world, &store).await?;
    commit(
        &mut rig.world,
        &store,
        rig.invitee,
        &declaration("ready", &selected),
    )
    .await?;
    assert_eq!(
        draft(&rig.world, rig.organizer, 1)?
            .consents
            .get(&rig.invitee)
            .ok_or("ready")?
            .state,
        ConsentState::Ready
    );
    persist_legacy(&mut rig.world, &store).await?;
    drop(rig);
    let mut rig = fixture(&store).await?;
    let restored = draft(&rig.world, rig.organizer, 1)?;
    assert_eq!(
        restored
            .consents
            .get(&rig.invitee)
            .ok_or("restarted declaration")?
            .state,
        ConsentState::Ready
    );
    assert!(
        rig.world
            .world()
            .module_state::<mush_sim::FocusState>()
            .ok_or("focus")?
            .focus_of_actor(rig.invitee)
            .is_none(),
        "restarted readiness does not restore physical control"
    );
    let report = text(
        &mut rig.world,
        rig.invitee,
        &format!("+sortie/plan {}:1", rig.organizer.0),
    );
    assert!(report.contains("ready declared for this exact revision"));
    assert!(report.contains("not established"));
    Ok(())
}

#[tokio::test]
async fn activation_requires_current_player_readiness_station_and_helm_lease() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let bay = seated_fixture(&mut rig, &store).await?;
    rig.world.world_mut().move_to(rig.organizer, bay)?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@asset/title #{}=#{}", rig.unit.0, rig.organizer.0),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    commit(
        &mut rig.world,
        &store,
        rig.organizer,
        &format!(
            "@unitgrant/issue #{}=#{} operate_crew",
            rig.unit.0, rig.invitee.0
        ),
    )
    .await?;
    let payload = plan(&rig);
    commit(
        &mut rig.world,
        &store,
        rig.organizer,
        &format!("+sortie/draft {payload}"),
    )
    .await?;
    let selected = draft(&rig.world, rig.organizer, 1)?;
    commit(
        &mut rig.world,
        &store,
        rig.invitee,
        &declaration("accept", &selected),
    )
    .await?;
    let request = submit(&mut rig.world, rig.organizer, "+sortie/activate 1:1=combat")?;
    assert!(
        stage(&mut rig.world, &store, &request).await.is_err(),
        "Accepted is not Ready"
    );
    commit(
        &mut rig.world,
        &store,
        rig.invitee,
        &format!("@station/handoff #{}/pilot", rig.unit.0),
    )
    .await?;
    install_due_handoff(&mut rig.world, &store).await?;
    commit(
        &mut rig.world,
        &store,
        rig.invitee,
        &declaration("ready", &selected),
    )
    .await?;
    assert!(
        rig.world
            .engine()
            .authority_witness(&mush_core::AuthorityWitnessRequest::ControlLease {
                provider: mush_core::OwnerId::new("sim")?,
                subject: rig.unit,
                channel: mush_core::ChannelId::new("helm").ok_or("helm channel")?,
                holder: mush_core::PrincipalId::Object { id: rig.invitee },
            })?
            .is_some(),
        "the assigned player's real helm lease must exist"
    );
    let request = submit(&mut rig.world, rig.organizer, "+sortie/activate 1:1=combat")?;
    stage(&mut rig.world, &store, &request).await?;
    rig.world
        .world_mut()
        .move_to(rig.invitee, mush_core::LIMBO)?;
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    rig.world.world_mut().move_to(rig.invitee, bay)?;
    persist_legacy(&mut rig.world, &store).await?;
    commit(
        &mut rig.world,
        &store,
        rig.organizer,
        "+sortie/activate 1:1=combat",
    )
    .await?;
    let accepted = text(
        &mut rig.world,
        rig.invitee,
        &format!("+sortie/active {}:1", rig.organizer.0),
    );
    assert!(accepted.contains("Prepared detachment"), "{accepted}");
    assert!(accepted.contains(&format!("player #{}", rig.invitee.0)));
    Ok(())
}

async fn field_fixture(store: &InMemoryStore) -> TestResult<(Rig, ObjId)> {
    field_operator_fixture(store, false).await
}

async fn field_operator_fixture(store: &InMemoryStore, npc: bool) -> TestResult<(Rig, ObjId)> {
    field_commander_fixture(store, npc, false).await
}
async fn field_commander_fixture(
    store: &InMemoryStore,
    npc: bool,
    commander: bool,
) -> TestResult<(Rig, ObjId)> {
    let mut rig = fixture(store).await?;
    let cab = rig
        .world
        .world_mut()
        .create(ObjKind::Room, "Field unit cockpit", GOD, None)?;
    rig.world.world_mut().attach_behavior(
        cab,
        BehaviorId::new("sim.interior")?,
        BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 0,
        },
    )?;
    rig.world.world_mut().attach_behavior(
        rig.unit,
        BehaviorId::new("sim.participant")?,
        BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 0,
        },
    )?;
    persist_legacy(&mut rig.world, store).await?;
    commit(
        &mut rig.world,
        store,
        GOD,
        &format!("@spatial/space #{}=moving:#{}", cab.0, rig.unit.0),
    )
    .await?;
    seated_fixture(&mut rig, store).await?;
    ordinary(&mut rig.world, rig.invitee, "+focus/release")?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/unassign #{}=pilot", rig.unit.0),
    )?;
    if npc {
        hire_field_operator(&mut rig, store, commander).await?;
    }
    let (hull, ceres, crew, site) = {
        let port = tacsim::port::coop::programme_port(rig.world.world()).ok_or("programme port")?;
        let row = port.map.hulls.first().ok_or("programme hull")?;
        let site = rig
            .world
            .world()
            .module_state::<tacsim::TacsimStore>()
            .ok_or("tacsim")?
            .port_maps()
            .find(|m| m.entry.as_str() == "ahuna_port")
            .and_then(|m| m.domain)
            .ok_or("field site domain")?;
        (
            ObjId(row.hull),
            port.map.domain.ok_or("Ceres domain")?,
            ObjId(
                *row.rooms
                    .get(&mush_core::CanonicalId::new("crew")?)
                    .ok_or("crew room")?,
            ),
            site,
        )
    };
    for command in [
        format!("@asset/transfer #{}=#{}", hull.0, rig.organizer.0),
        format!("@spatial/place/domain #{}={ceres}", hull.0),
        // Staff constructs the real bay-equipped variant; no fake HullView or
        // placement/lease mirror substitutes for its authored port interior.
        format!("@unit/remove #{}", hull.0),
        format!("@unit/create #{}=tacsim.unit.dray_p", hull.0),
    ] {
        ordinary(&mut rig.world, GOD, &command)?;
    }
    rig.carrier = hull;
    let (bay, slot) = {
        let view =
            tacsim::port::ship::hull_view(rig.world.world(), hull).ok_or("real hull view")?;
        let (bay, room) = view.bay().ok_or("authored physical bay")?;
        (room, bay.slot.id.to_string())
    };
    for command in [
        format!("@spatial/slot #{}=berths,1", cab.0),
        format!(
            "@spatial/place/interior #{}=#{},{}",
            rig.unit.0, bay.0, slot
        ),
        format!(
            "@spatial/station #{}=pilot,pilot,channels:helm,space:#{}",
            rig.unit.0, cab.0
        ),
        format!("@asset/title #{}=#{}", rig.unit.0, rig.organizer.0),
    ] {
        ordinary(&mut rig.world, GOD, &command)?;
    }
    if npc {
        ordinary(
            &mut rig.world,
            rig.organizer,
            &format!("+sortie/board #{}", rig.invitee.0),
        )?;
    } else {
        ordinary(
            &mut rig.world,
            GOD,
            &format!("@spatial/assign #{}=pilot,#{}", rig.unit.0, rig.invitee.0),
        )?;
    }
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/place/domain #{}={site}", hull.0),
    )?;
    rig.world.world_mut().move_to(rig.organizer, crew)?;
    if !npc {
        rig.world.world_mut().move_to(rig.invitee, cab)?;
        ordinary(
            &mut rig.world,
            rig.invitee,
            &format!("+focus #{}/pilot", rig.unit.0),
        )?;
    }
    persist_legacy(&mut rig.world, store).await?;
    commit(
        &mut rig.world,
        store,
        rig.organizer,
        &format!(
            "@unitgrant/issue #{}=#{} operate_crew",
            rig.unit.0, rig.invitee.0
        ),
    )
    .await?;
    let mut payload = plan(&rig);
    if npc {
        payload["slots"][0]["controller"]["kind"] = json!("npc");
        if commander {
            payload["npc_commander"] = json!(rig.invitee.0);
        }
    }
    commit(
        &mut rig.world,
        store,
        rig.organizer,
        &format!("+sortie/draft {payload}"),
    )
    .await?;
    if !npc {
        let selected = draft(&rig.world, rig.organizer, 1)?;
        commit(
            &mut rig.world,
            store,
            rig.invitee,
            &declaration("accept", &selected),
        )
        .await?;
        commit(
            &mut rig.world,
            store,
            rig.invitee,
            &format!("@station/handoff #{}/pilot", rig.unit.0),
        )
        .await?;
        install_due_handoff(&mut rig.world, store).await?;
        commit(
            &mut rig.world,
            store,
            rig.invitee,
            &declaration("ready", &selected),
        )
        .await?;
    }
    commit(
        &mut rig.world,
        store,
        rig.organizer,
        "+sortie/activate 1:1=combat",
    )
    .await?;
    Ok((rig, cab))
}

async fn hire_field_operator(rig: &mut Rig, store: &InMemoryStore, commander: bool) -> TestResult {
    use tacsim::detachment::crew::{CrewDuty, HireTerms, NpcMissionScope, OFFER_ATTR};
    let key = mush_atlas::RecordKey::new(
        mush_core::CanonicalId::new("neighborhood.ceres_freeport")?,
        mush_core::CanonicalId::new("room.c10_slack_line")?,
    );
    let room =
        tacsim::port::apply::room_by_key(rig.world.world(), &key).ok_or("actual crew bar")?;
    rig.world.world_mut().move_to(rig.organizer, room)?;
    let npc =
        rig.world
            .world_mut()
            .create(ObjKind::Npc, "Contracted field pilot", GOD, Some(room))?;
    let terms = HireTerms {
        schema: 1,
        port: mush_core::CanonicalId::new("ceres")?,
        mission_scope: if commander {
            NpcMissionScope::DetachmentCommander
        } else {
            NpcMissionScope::SubordinateOperator
        },
        duties: std::collections::BTreeSet::from([CrewDuty::Combat]),
    };
    rig.world.world_mut().set_attr(
        npc,
        OFFER_ATTR,
        mush_core::Attr::json(serde_json::to_value(&terms)?, GOD),
    )?;
    persist_legacy(&mut rig.world, store).await?;
    commit(
        &mut rig.world,
        store,
        rig.organizer,
        &format!("+sortie/hire #{}={}", npc.0, terms.digest()?.to_hex()),
    )
    .await?;
    rig.invitee = npc;
    Ok(())
}
fn flight(
    world: &TestWorld,
    unit: ObjId,
    ordinal: u64,
) -> TestResult<tacsim::detachment::sortie::Flight> {
    let h = world
        .world()
        .module_state::<tacsim::detachment::sortie::SortieHandles>()
        .ok_or("flight handles")?;
    world
        .engine()
        .action_state()
        .get(
            &h.flights,
            &tacsim::detachment::sortie::FlightKey { unit, ordinal },
        )?
        .cloned()
        .ok_or_else(|| "flight missing".into())
}
async fn fire_flight(
    world: &mut TestWorld,
    store: &InMemoryStore,
    due: i64,
) -> TestResult<Box<mush_core::AuthoritativeSubmit>> {
    let rate = i64::from(world.world().world_timeline().current().rate.max(1));
    let ahead = due.saturating_sub(world.world().world_ic_now().at).max(0);
    let real = ((ahead + rate - 1) / rate).max(5);
    let now = world.world().now().saturating_add(real);
    world.set_now(now);
    world.fire_due(now.saturating_mul(1000));
    persist_legacy(world, store).await?;
    let mut wanted = None;
    while let Some(request) = world.engine_mut().take_authoritative_submit() {
        if request.action.key().id.as_str() == "tacsim_detachment_flight" {
            wanted = Some(request);
        }
    }
    wanted.ok_or_else(|| "no flight completion queued".into())
}
async fn land_request(
    world: &mut TestWorld,
    store: &InMemoryStore,
    request: &mush_core::AuthoritativeSubmit,
) -> TestResult {
    stage(world, store, request).await?;
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()?
        .ok_or("flight bundle")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    world.engine_mut().install_composed_action(installation)?;
    Ok(())
}

#[tokio::test]
async fn physical_detachment_launch_return_relaunch_and_end_preserve_real_unit_and_epochs()
-> TestResult {
    use tacsim::detachment::sortie::Phase;
    let store = InMemoryStore::new();
    let (mut rig, cab) = field_fixture(&store).await?;
    let organizer = rig.organizer;
    let before = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("spatial")?
        .parent_of(rig.unit);
    let request = submit(&mut rig.world, organizer, "+sortie/launch 1:1=all")?;
    stage(&mut rig.world, &store, &request).await?;
    assert_eq!(
        rig.world
            .world()
            .module_state::<mush_sim::SpatialState>()
            .ok_or("spatial")?
            .parent_of(rig.unit),
        before,
        "prepare cannot move a unit"
    );
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("launch bundle")?
        .into_parts();
    assert!(flight(&rig.world, rig.unit, 1).is_err());
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    rig.world
        .engine_mut()
        .install_composed_action(installation)?;
    let departure = flight(&rig.world, rig.unit, 1)?;
    assert_eq!(departure.phase, Phase::Departing);
    assert!(departure.epoch.is_none());
    let finish = fire_flight(&mut rig.world, &store, departure.due.at).await?;
    assert!(
        matches!(rig.world.world().module_state::<mush_sim::SpatialState>().ok_or("spatial")?.parent_of(rig.unit),mush_sim::SpatialParent::Direct{domain,..} if domain==departure.domain)
    );
    assert_eq!(
        rig.world.world().get(rig.invitee).and_then(|o| o.location),
        Some(cab),
        "operator travels inside the real unit"
    );
    assert_eq!(
        flight(&rig.world, rig.unit, 1)?.phase,
        Phase::Departing,
        "placement alone is not a confirmed history row"
    );
    land_request(&mut rig.world, &store, &finish).await?;
    let field = flight(&rig.world, rig.unit, 1)?;
    let epoch = field.epoch.as_ref().ok_or("actual hull epoch")?.id;
    assert_eq!(field.phase, Phase::Field);
    assert!(
        !field.epoch.as_ref().ok_or("epoch")?.unrecorded,
        "actual black box participant must append the opening"
    );
    let duplicate = text(&mut rig.world, organizer, "+sortie/launch 1:1=all");
    assert!(duplicate.contains("already departed"), "{duplicate}");
    assert!(rig.world.engine_mut().take_authoritative_submit().is_none());
    commit(&mut rig.world, &store, organizer, "+sortie/recover 1:1=1").await?;
    let recovering = flight(&rig.world, rig.unit, 1)?;
    let done = fire_flight(&mut rig.world, &store, recovering.due.at).await?;
    land_request(&mut rig.world, &store, &done).await?;
    let returned = flight(&rig.world, rig.unit, 1)?;
    assert_eq!(returned.phase, Phase::Returned);
    assert_eq!(returned.epoch.as_ref().ok_or("retained epoch")?.id, epoch);
    assert!(returned.closed.as_ref().ok_or("return")?.operator_aboard);
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let again = flight(&rig.world, rig.unit, 2)?;
    let done = fire_flight(&mut rig.world, &store, again.due.at).await?;
    land_request(&mut rig.world, &store, &done).await?;
    let again = flight(&rig.world, rig.unit, 2)?;
    assert_ne!(again.epoch.as_ref().ok_or("next epoch")?.id, epoch);
    commit(&mut rig.world, &store, organizer, "+sortie/end 1:1").await?;
    assert_eq!(flight(&rig.world, rig.unit, 2)?.phase, Phase::Field);
    let report = text(&mut rig.world, organizer, "+sortie/operation 1:1");
    assert!(
        report.contains("unresolved, physical epoch remains open"),
        "{report}"
    );
    let hidden = text(
        &mut rig.world,
        rig.stranger,
        &format!("+sortie/history #{}:2", rig.unit.0),
    );
    assert!(!hidden.contains("Hull epoch"), "{hidden}");
    Ok(())
}

#[tokio::test]
async fn physical_departure_confirmation_survives_lost_ack_and_restart() -> TestResult {
    use tacsim::detachment::sortie::Phase;
    let store = InMemoryStore::new();
    let (mut rig, cab) = field_fixture(&store).await?;
    let organizer = rig.organizer;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let departure = flight(&rig.world, rig.unit, 1)?;
    let request = fire_flight(&mut rig.world, &store, departure.due.at).await?;
    let ingress = stage(&mut rig.world, &store, &request).await?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("confirmation bundle")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert_eq!(flight(&rig.world, rig.unit, 1)?.phase, Phase::Departing);
    drop(installation);
    drop(rig);
    let rig = fixture(&store).await?;
    let restored = flight(&rig.world, rig.unit, 1)?;
    assert_eq!(restored.phase, Phase::Field);
    assert!(!restored.epoch.as_ref().ok_or("restored epoch")?.unrecorded);
    assert_eq!(
        rig.world.world().get(rig.invitee).and_then(|o| o.location),
        Some(cab)
    );
    assert!(
        matches!(rig.world.world().module_state::<mush_sim::SpatialState>()
        .ok_or("spatial")?.parent_of(rig.unit), mush_sim::SpatialParent::Direct{domain,..}
        if domain == restored.domain)
    );
    let ActionPreflight::Finalized(retry) = ActionCoordinator::preflight(&store, ingress).await?
    else {
        return Err("confirmation retry lost its durable receipt".into());
    };
    assert_eq!(retry.token(), bundle.token());
    assert_eq!(retry.outcome(), receipt.outcome());
    Ok(())
}

#[tokio::test]
async fn physical_launch_rechecks_presence_and_cancel_retains_history_without_an_epoch()
-> TestResult {
    use tacsim::detachment::sortie::Phase;
    let store = InMemoryStore::new();
    let (mut rig, cab) = field_fixture(&store).await?;
    let organizer = rig.organizer;
    let request = submit(&mut rig.world, organizer, "+sortie/launch 1:1=1")?;
    stage(&mut rig.world, &store, &request).await?;
    rig.world
        .world_mut()
        .move_to(rig.invitee, mush_core::LIMBO)?;
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    assert!(flight(&rig.world, rig.unit, 1).is_err());
    rig.world.world_mut().move_to(rig.invitee, cab)?;
    rig.world.set_now(rig.world.world().now() + 1);
    persist_legacy(&mut rig.world, &store).await?;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    commit(
        &mut rig.world,
        &store,
        organizer,
        "+sortie/cancel-launch 1:1=1",
    )
    .await?;
    let cancelled = flight(&rig.world, rig.unit, 1)?;
    assert_eq!(cancelled.phase, Phase::Cancelled);
    assert!(cancelled.epoch.is_none());
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    assert_eq!(flight(&rig.world, rig.unit, 2)?.phase, Phase::Departing);
    assert_eq!(flight(&rig.world, rig.unit, 1)?, cancelled);
    Ok(())
}

#[tokio::test]
async fn admitted_physical_launch_survives_rate_change_and_waits_during_freeze() -> TestResult {
    use tacsim::detachment::sortie::Phase;
    let store = InMemoryStore::new();
    let (mut rig, _) = field_fixture(&store).await?;
    let organizer = rig.organizer;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let departure = flight(&rig.world, rig.unit, 1)?;
    ordinary(&mut rig.world, GOD, "@tacsim/time rate=24")?;
    ordinary(&mut rig.world, GOD, "@tacsim/time freeze")?;
    let before = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("spatial")?
        .parent_of(rig.unit);
    let now = rig.world.world().now() + 600;
    rig.world.set_now(now);
    rig.world.fire_due(now * 1000);
    while let Some(request) = rig.world.engine_mut().take_authoritative_submit() {
        assert_ne!(request.action.key().id.as_str(), "tacsim_detachment_flight");
    }
    assert_eq!(
        rig.world
            .world()
            .module_state::<mush_sim::SpatialState>()
            .ok_or("spatial")?
            .parent_of(rig.unit),
        before
    );
    ordinary(&mut rig.world, GOD, "@tacsim/time resume")?;
    let request = fire_flight(&mut rig.world, &store, departure.due.at).await?;
    land_request(&mut rig.world, &store, &request).await?;
    assert_eq!(flight(&rig.world, rig.unit, 1)?.phase, Phase::Field);
    Ok(())
}

#[tokio::test]
async fn physical_launch_final_take_refuses_a_revoked_deploy_right() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _) = field_fixture(&store).await?;
    let organizer = rig.organizer;
    let request = submit(&mut rig.world, organizer, "+sortie/launch 1:1=1")?;
    stage(&mut rig.world, &store, &request).await?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@asset/transfer #{}=#{}", rig.unit.0, rig.stranger.0),
    )?;
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    assert!(flight(&rig.world, rig.unit, 1).is_err());
    let handles = rig
        .world
        .world()
        .module_state::<tacsim::detachment::sortie::SortieHandles>()
        .ok_or("flight handles")?;
    assert!(
        rig.world
            .engine()
            .action_state()
            .get(&handles.index, &())?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn physical_departure_before_confirmation_reconciles_after_restart() -> TestResult {
    use tacsim::detachment::sortie::Phase;
    let store = InMemoryStore::new();
    let (mut rig, _) = field_fixture(&store).await?;
    let organizer = rig.organizer;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let departure = flight(&rig.world, rig.unit, 1)?;
    let _lost_request = fire_flight(&mut rig.world, &store, departure.due.at).await?;
    let now = rig.world.world().now();
    assert_eq!(flight(&rig.world, rig.unit, 1)?.phase, Phase::Departing);
    drop(rig);
    let mut rig = fixture(&store).await?;
    rig.world.set_now(now);
    assert_eq!(flight(&rig.world, rig.unit, 1)?.phase, Phase::Departing);
    let before = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("spatial")?
        .parent_of(rig.unit);
    let request = fire_flight(&mut rig.world, &store, departure.due.at).await?;
    land_request(&mut rig.world, &store, &request).await?;
    assert_eq!(flight(&rig.world, rig.unit, 1)?.phase, Phase::Field);
    assert_eq!(
        rig.world
            .world()
            .module_state::<mush_sim::SpatialState>()
            .ok_or("spatial")?
            .parent_of(rig.unit),
        before
    );
    Ok(())
}

fn actual_helm(world: &TestWorld, unit: ObjId, actor: ObjId) -> TestResult<bool> {
    Ok(world
        .engine()
        .authority_witness(&mush_core::AuthorityWitnessRequest::ControlLease {
            provider: mush_core::OwnerId::new("sim")?,
            subject: unit,
            channel: mush_core::ChannelId::new("helm").ok_or("helm")?,
            holder: mush_core::PrincipalId::Object { id: actor },
        })?
        .is_some())
}

#[tokio::test]
async fn hired_npc_boards_takes_actual_unit_control_and_departs_without_proxy_focus() -> TestResult
{
    use tacsim::detachment::sortie::Phase;
    let store = InMemoryStore::new();
    let (mut rig, cab) = field_operator_fixture(&store, true).await?;
    let organizer = rig.organizer;
    assert!(!actual_helm(&rig.world, rig.unit, rig.invitee)?);
    assert!(
        rig.world
            .world()
            .module_state::<mush_sim::FocusState>()
            .ok_or("focus")?
            .focus_of_actor(rig.invitee)
            .is_none()
    );
    let request = submit(&mut rig.world, organizer, "+sortie/seat 1:1=1")?;
    assert_eq!(
        request.actor, rig.invitee,
        "the existing handoff belongs to the NPC"
    );
    assert_eq!(
        rig.world.world().get(rig.invitee).and_then(|o| o.location),
        Some(cab)
    );
    assert!(
        !actual_helm(&rig.world, rig.unit, rig.invitee)?,
        "physical seating is not a lease"
    );
    persist_legacy(&mut rig.world, &store).await?;
    land_request(&mut rig.world, &store, &request).await?;
    assert!(
        !actual_helm(&rig.world, rig.unit, rig.invitee)?,
        "a pending request is not a lease"
    );
    let (npc, carrier, now) = (rig.invitee, rig.carrier, rig.world.world().now());
    drop(rig);
    let mut rig = fixture(&store).await?;
    rig.invitee = npc;
    rig.carrier = carrier;
    rig.world.set_now(now);
    assert_eq!(
        rig.world.world().get(npc).and_then(|o| o.location),
        Some(cab)
    );
    assert!(
        rig.world
            .world()
            .module_state::<mush_sim::FocusState>()
            .ok_or("focus")?
            .focus_of_actor(npc)
            .is_none()
    );
    install_due_handoff(&mut rig.world, &store).await?;
    assert!(actual_helm(&rig.world, rig.unit, rig.invitee)?);
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let departure = flight(&rig.world, rig.unit, 1)?;
    let request = fire_flight(&mut rig.world, &store, departure.due.at).await?;
    land_request(&mut rig.world, &store, &request).await?;
    let field = flight(&rig.world, rig.unit, 1)?;
    assert_eq!(field.phase, Phase::Field);
    assert_eq!(
        field.epoch.as_ref().ok_or("NPC epoch")?.operator,
        rig.invitee
    );
    assert_eq!(
        rig.world.world().get(rig.invitee).and_then(|o| o.location),
        Some(cab)
    );
    commit(&mut rig.world, &store, organizer, "+sortie/recover 1:1=1").await?;
    let returning = flight(&rig.world, rig.unit, 1)?;
    let request = fire_flight(&mut rig.world, &store, returning.due.at).await?;
    land_request(&mut rig.world, &store, &request).await?;
    assert_eq!(flight(&rig.world, rig.unit, 1)?.phase, Phase::Returned);
    Ok(())
}

#[tokio::test]
async fn hired_npc_station_request_and_apply_recheck_current_physical_console() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, cab) = field_operator_fixture(&store, true).await?;
    let organizer = rig.organizer;
    let crew = rig
        .world
        .world()
        .get(rig.invitee)
        .and_then(|o| o.location)
        .ok_or("crew compartment")?;
    let crew = mush_sim::anchored_space_of(rig.world.world(), crew).ok_or("carrier interior")?;
    let request = submit(&mut rig.world, organizer, "+sortie/seat 1:1=1")?;
    persist_legacy(&mut rig.world, &store).await?;
    stage(&mut rig.world, &store, &request).await?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/place/interior #{}=#{}", rig.invitee.0, crew.0),
    )?;
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    assert!(!actual_helm(&rig.world, rig.unit, rig.invitee)?);
    rig.world.set_now(rig.world.world().now() + 1);
    let request = submit(&mut rig.world, organizer, "+sortie/seat 1:1=1")?;
    persist_legacy(&mut rig.world, &store).await?;
    land_request(&mut rig.world, &store, &request).await?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/place/interior #{}=#{}", rig.invitee.0, crew.0),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    assert!(install_due_handoff(&mut rig.world, &store).await.is_err());
    assert!(!actual_helm(&rig.world, rig.unit, rig.invitee)?);
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/place/interior #{}=#{}", rig.invitee.0, cab.0),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    install_due_handoff(&mut rig.world, &store).await?;
    assert!(actual_helm(&rig.world, rig.unit, rig.invitee)?);
    Ok(())
}

#[tokio::test]
async fn hired_npc_seating_requires_current_real_unit_use_permission() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _) = field_operator_fixture(&store, true).await?;
    let organizer = rig.organizer;
    let before = rig.world.world().get(rig.invitee).and_then(|o| o.location);
    commit(&mut rig.world, &store, organizer, "@unitgrant/revoke 1").await?;
    let refused = text(&mut rig.world, organizer, "+sortie/seat 1:1=1");
    assert!(
        refused.contains("OperateCrew permissions are required"),
        "{refused}"
    );
    assert_eq!(
        rig.world.world().get(rig.invitee).and_then(|o| o.location),
        before
    );
    assert!(!actual_helm(&rig.world, rig.unit, rig.invitee)?);
    assert!(rig.world.engine_mut().take_authoritative_submit().is_none());
    Ok(())
}

fn field_roster(
    world: &TestWorld,
    organizer: ObjId,
) -> TestResult<mush_tactics::detachment::roster::DetachmentRoster> {
    let h = world
        .world()
        .module_state::<tacsim::detachment::sortie::SortieHandles>()
        .ok_or("flight handles")?;
    let op = world
        .engine()
        .action_state()
        .get(
            &h.operations,
            &tacsim::detachment::sortie::OperationId {
                organizer,
                draft: PlanId(1),
                activation: 1,
            },
        )?
        .ok_or("operation")?;
    Ok(mush_tactics::detachment::roster::DetachmentRoster::restore(
        op.command.as_deref().ok_or("field command")?,
    )?)
}

fn field_orders(
    world: &TestWorld,
    organizer: ObjId,
) -> TestResult<tacsim::detachment::sortie::FieldOrders> {
    let h = world
        .world()
        .module_state::<tacsim::detachment::sortie::SortieHandles>()
        .ok_or("flight handles")?;
    Ok(world
        .engine()
        .action_state()
        .get(
            &h.operations,
            &tacsim::detachment::sortie::OperationId {
                organizer,
                draft: PlanId(1),
                activation: 1,
            },
        )?
        .and_then(|o| o.orders.as_deref())
        .cloned()
        .ok_or("field orders")?)
}

fn command_grant(world: &mut TestWorld, organizer: ObjId) -> TestResult<String> {
    let report = text(world, organizer, "+sortie/command-grants 1:1");
    report
        .lines()
        .find_map(|l| {
            l.find("@unitgrant/json ")
                .map(|start| l[start..].to_owned())
        })
        .ok_or_else(|| format!("no role prefill: {report}").into())
}

#[tokio::test]
async fn ic_mission_role_grants_bind_the_accepted_roster_without_personal_command_rights()
-> TestResult {
    use mush_tactics::detachment::{OrderBook, Slot};
    let store = InMemoryStore::new();
    let (mut rig, _) = field_fixture(&store).await?;
    let (organizer, player, unit) = (rig.organizer, rig.invitee, rig.unit);
    rig.world.connect(player);
    let grant = command_grant(&mut rig.world, organizer)?;
    let raw = grant.strip_prefix("@unitgrant/json ").ok_or("draft")?;
    let mut wrong: Value = serde_json::from_str(raw)?;
    wrong["grantee"]["CurrentLeader"]["accepted_roster_hash"][0] = json!(255);
    if wrong == serde_json::from_str::<Value>(raw)? {
        wrong["grantee"]["CurrentLeader"]["accepted_roster_hash"][0] = json!(254);
    }
    commit(
        &mut rig.world,
        &store,
        organizer,
        &format!("@unitgrant/json {wrong}"),
    )
    .await?;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let due = flight(&rig.world, unit, 1)?.due.at;
    let finish = fire_flight(&mut rig.world, &store, due).await?;
    land_request(&mut rig.world, &store, &finish).await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    let roster = field_roster(&rig.world, organizer)?;
    let command = format!(
        "ORDER DT:{}:{}:{} 1 HOLD",
        roster.id().0,
        roster.revision(),
        roster.term()
    );
    assert!(
        submit(&mut rig.world, player, &command).is_err(),
        "a different accepted digest cannot authorize this roster"
    );
    assert!(field_orders(&rig.world, organizer).is_err());
    commit(&mut rig.world, &store, organizer, &grant).await?;
    let check = text(
        &mut rig.world,
        player,
        &format!("@unitgrant/check #{}=command_assigned_ai", unit.0),
    );
    assert!(
        check.contains("unavailable"),
        "mission roles are not personal grants: {check}"
    );
    commit(&mut rig.world, &store, player, &command).await?;
    let now = rig.world.world().world_ic_now().at;
    let poll = fire_flight(&mut rig.world, &store, now).await?;
    land_request(&mut rig.world, &store, &poll).await?;
    let slot = Slot::new(1)?;
    let saved = field_orders(&rig.world, organizer)?;
    let roster = field_roster(&rig.world, organizer)?;
    let book = OrderBook::restore(&saved.book, &roster)?;
    let original = book
        .delivered(slot)
        .ok_or("role-authorized arrival")?
        .order
        .sequence;
    // The declaration and its exact binding survive restart, with no leader or
    // current permission cached in a World mirror.
    persist_legacy(&mut rig.world, &store).await?;
    let now = rig.world.world().now();
    drop(rig);
    let (mut restarted, cab) = {
        let rig = fixture(&store).await?;
        let cab = rig
            .world
            .world()
            .get(player)
            .and_then(|p| p.location)
            .ok_or("cab")?;
        (rig, cab)
    };
    restarted.world.set_now(now);
    restarted.world.connect(player);
    *restarted
        .world
        .world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .ok_or("account oracle")? = mush_sim::AccountOracle::new(Arc::new(PilotAccount {
        actor: player,
        account: AccountId::mint("PlanningPilot", 1)?,
    }));
    ordinary(
        &mut restarted.world,
        player,
        &format!("+focus #{}/pilot", unit.0),
    )?;
    assert_eq!(
        restarted.world.world().get(player).and_then(|p| p.location),
        Some(cab)
    );
    persist_legacy(&mut restarted.world, &store).await?;
    commit(
        &mut restarted.world,
        &store,
        organizer,
        "+sortie/refresh 1:1",
    )
    .await?;
    let roster = field_roster(&restarted.world, organizer)?;
    let command = format!(
        "ORDER DT:{}:{}:{} 1 ADVANCE 4,5",
        roster.id().0,
        roster.revision(),
        roster.term()
    );
    commit(&mut restarted.world, &store, player, &command).await?;
    commit(
        &mut restarted.world,
        &store,
        organizer,
        "@unitgrant/revoke 3",
    )
    .await?;
    let now = restarted.world.world().world_ic_now().at;
    let poll = fire_flight(&mut restarted.world, &store, now).await?;
    land_request(&mut restarted.world, &store, &poll).await?;
    let saved = field_orders(&restarted.world, organizer)?;
    let book = OrderBook::restore(&saved.book, &field_roster(&restarted.world, organizer)?)?;
    assert!(book.pending(slot).is_none());
    assert_eq!(
        book.delivered(slot).ok_or("standing")?.order.sequence,
        original
    );
    Ok(())
}

#[tokio::test]
async fn ic_mission_role_can_authorize_the_actual_nominated_npc_without_electing_an_absent_one()
-> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _) = field_commander_fixture(&store, true, true).await?;
    let (organizer, npc, unit) = (rig.organizer, rig.invitee, rig.unit);
    let grant = command_grant(&mut rig.world, organizer)?;
    commit(&mut rig.world, &store, organizer, &grant).await?;
    let seating = submit(&mut rig.world, organizer, "+sortie/seat 1:1=1")?;
    persist_legacy(&mut rig.world, &store).await?;
    land_request(&mut rig.world, &store, &seating).await?;
    install_due_handoff(&mut rig.world, &store).await?;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert!(
        field_roster(&rig.world, organizer)?.leader().is_none(),
        "the granted NPC is still aboard during ramp transit"
    );
    let due = flight(&rig.world, unit, 1)?.due.at;
    let finish = fire_flight(&mut rig.world, &store, due).await?;
    land_request(&mut rig.world, &store, &finish).await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert_eq!(
        field_roster(&rig.world, organizer)?
            .leader()
            .map(|l| l.character),
        Some(npc)
    );
    commit(&mut rig.world, &store, organizer, "@unitgrant/revoke 2").await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert!(field_roster(&rig.world, organizer)?.leader().is_none());
    assert_eq!(
        flight(&rig.world, unit, 1)?.phase,
        tacsim::detachment::sortie::Phase::Field
    );
    Ok(())
}

#[tokio::test]
async fn ic_field_orders_use_real_transport_and_advisory_responses() -> TestResult {
    use mush_propagation::{EnvelopeState, PropagationStore};
    use mush_tactics::detachment::{OrderBook, Slot};
    let store = InMemoryStore::new();
    let (mut rig, _) = field_fixture(&store).await?;
    let (organizer, player, unit) = (rig.organizer, rig.invitee, rig.unit);
    rig.world.connect(player);
    commit(
        &mut rig.world,
        &store,
        organizer,
        &format!(
            "@unitgrant/issue #{}=#{} command_assigned_ai",
            unit.0, player.0
        ),
    )
    .await?;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let due = flight(&rig.world, unit, 1)?.due.at;
    let finish = fire_flight(&mut rig.world, &store, due).await?;
    land_request(&mut rig.world, &store, &finish).await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    let roster = field_roster(&rig.world, organizer)?;
    let fence = format!(
        "DT:{}:{}:{}",
        roster.id().0,
        roster.revision(),
        roster.term()
    );
    let before = mush_sim::pose_of(rig.world.world(), unit).ok_or("field pose")?;
    commit(
        &mut rig.world,
        &store,
        player,
        &format!("ORDER {fence} 1 HOLD"),
    )
    .await?;
    let orders = field_orders(&rig.world, organizer)?;
    let slot = Slot::new(1)?;
    let outgoing = *orders
        .outbound
        .get(&slot)
        .ok_or("actual outbound reference")?;
    assert_eq!(
        rig.world
            .world()
            .module_state::<PropagationStore>()
            .and_then(|s| s.envelope(outgoing))
            .ok_or("actual envelope")?
            .state(),
        EnvelopeState::Queued
    );
    assert!(
        OrderBook::restore(&orders.book, &field_roster(&rig.world, organizer)?)?
            .delivered(slot)
            .is_none(),
        "queued is not delivered"
    );
    let now = rig.world.world().world_ic_now().at;
    let poll = fire_flight(&mut rig.world, &store, now).await?;
    land_request(&mut rig.world, &store, &poll).await?;
    let orders = field_orders(&rig.world, organizer)?;
    let book = OrderBook::restore(&orders.book, &field_roster(&rig.world, organizer)?)?;
    let delivered = book.delivered(slot).ok_or("actual delivery")?;
    assert!(delivered.acknowledgement.is_none());
    assert!(delivered.attempted_at_ms.is_none() && delivered.completed_at_ms.is_none());
    let sequence = delivered.order.sequence;
    commit(
        &mut rig.world,
        &store,
        player,
        &format!(
            "WILCO DT:{}:{} {}",
            roster.id().0,
            roster.revision(),
            sequence
        ),
    )
    .await?;
    let report = text(
        &mut rig.world,
        player,
        &format!("+sortie/orders {}:1:1", organizer.0),
    );
    assert!(report.contains("no reply confirmed"), "{report}");
    assert!(
        !report.contains("WILCO received"),
        "a pending return packet must reveal no acknowledgement"
    );
    let now = rig.world.world().world_ic_now().at;
    let _ = fire_flight(&mut rig.world, &store, now).await;
    let report = text(
        &mut rig.world,
        player,
        &format!("+sortie/orders {}:1:1", organizer.0),
    );
    assert!(report.contains("WILCO received"), "{report}");
    assert_eq!(
        mush_sim::pose_of(rig.world.world(), unit),
        Some(before),
        "orders and WILCO never drive a player's unit"
    );
    let orders = field_orders(&rig.world, organizer)?;
    let book = OrderBook::restore(&orders.book, &field_roster(&rig.world, organizer)?)?;
    assert!(
        book.delivered(slot)
            .ok_or("delivered")?
            .attempted_at_ms
            .is_none()
    );
    assert!(
        book.delivered(slot)
            .ok_or("delivered")?
            .completed_at_ms
            .is_none()
    );
    // Repeat replies are distinct actual packets. Neither boot nor the generic
    // mail/channel reconciler may discard them as duplicate external handoffs.
    commit(
        &mut rig.world,
        &store,
        player,
        &format!(
            "WILCO DT:{}:{} {}",
            roster.id().0,
            roster.revision(),
            sequence
        ),
    )
    .await?;
    let repeated = field_orders(&rig.world, organizer)?
        .replies
        .get(&slot)
        .ok_or("reply")?
        .envelope;
    commit(
        &mut rig.world,
        &store,
        player,
        &format!("ORDER {fence} 1 ADVANCE 4,5"),
    )
    .await?;
    let pending = *field_orders(&rig.world, organizer)?
        .outbound
        .get(&slot)
        .ok_or("pending")?;
    persist_legacy(&mut rig.world, &store).await?;
    let now = rig.world.world().now();
    drop(rig);
    let mut restarted = fixture(&store).await?;
    restarted.world.set_now(now);
    let orders = field_orders(&restarted.world, organizer)?;
    let roster = field_roster(&restarted.world, organizer)?;
    let book = OrderBook::restore(&orders.book, &roster)?;
    assert!(book.pending(slot).is_some());
    assert_eq!(
        book.delivered(slot).ok_or("standing")?.order.sequence,
        sequence
    );
    for id in [outgoing, repeated, pending] {
        assert!(
            restarted
                .world
                .world()
                .module_state::<PropagationStore>()
                .and_then(|s| s.envelope(id))
                .is_some(),
            "actual packet {id} survives boot"
        );
    }
    // Restarted session/control facts are unavailable. A saved term cannot
    // activate the pending replacement; its older delivered history survives.
    let now = restarted.world.world().world_ic_now().at;
    let poll = fire_flight(&mut restarted.world, &store, now).await?;
    land_request(&mut restarted.world, &store, &poll).await?;
    let orders = field_orders(&restarted.world, organizer)?;
    let book = OrderBook::restore(&orders.book, &field_roster(&restarted.world, organizer)?)?;
    assert!(book.pending(slot).is_none());
    assert_eq!(
        book.delivered(slot).ok_or("standing")?.order.sequence,
        sequence
    );
    Ok(())
}

#[tokio::test]
async fn ic_field_order_refusal_at_final_take_publishes_neither_source_nor_packet() -> TestResult {
    use mush_propagation::PropagationStore;
    let store = InMemoryStore::new();
    let (mut rig, _) = field_fixture(&store).await?;
    let (organizer, player, unit) = (rig.organizer, rig.invitee, rig.unit);
    rig.world.connect(player);
    commit(
        &mut rig.world,
        &store,
        organizer,
        &format!(
            "@unitgrant/issue #{}=#{} command_assigned_ai",
            unit.0, player.0
        ),
    )
    .await?;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let due = flight(&rig.world, unit, 1)?.due.at;
    let finish = fire_flight(&mut rig.world, &store, due).await?;
    land_request(&mut rig.world, &store, &finish).await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    let roster = field_roster(&rig.world, organizer)?;
    let count = rig
        .world
        .world()
        .module_state::<PropagationStore>()
        .ok_or("transport")?
        .envelopes()
        .count();
    let request = submit(
        &mut rig.world,
        player,
        &format!(
            "ORDER DT:{}:{}:{} 1 HOLD",
            roster.id().0,
            roster.revision(),
            roster.term()
        ),
    )?;
    stage(&mut rig.world, &store, &request).await?;
    // Invalidate an actual physical console fact after owner preparation.
    rig.world.world_mut().move_to(player, mush_core::LIMBO)?;
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    assert!(field_orders(&rig.world, organizer).is_err());
    assert_eq!(
        rig.world
            .world()
            .module_state::<PropagationStore>()
            .ok_or("transport")?
            .envelopes()
            .count(),
        count
    );
    Ok(())
}

#[tokio::test]
async fn ic_field_revocation_and_expiry_leave_the_confirmed_standing_order_intact() -> TestResult {
    use mush_tactics::detachment::{OrderBook, Slot};
    use tacsim::detachment::sortie::FieldObjective;
    let store = InMemoryStore::new();
    let (mut rig, _) = field_fixture(&store).await?;
    let (organizer, player, unit) = (rig.organizer, rig.invitee, rig.unit);
    rig.world.connect(player);
    commit(
        &mut rig.world,
        &store,
        organizer,
        &format!(
            "@unitgrant/issue #{}=#{} command_assigned_ai",
            unit.0, player.0
        ),
    )
    .await?;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let due = flight(&rig.world, unit, 1)?.due.at;
    let finish = fire_flight(&mut rig.world, &store, due).await?;
    land_request(&mut rig.world, &store, &finish).await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    let roster = field_roster(&rig.world, organizer)?;
    let fence = format!(
        "DT:{}:{}:{}",
        roster.id().0,
        roster.revision(),
        roster.term()
    );
    commit(
        &mut rig.world,
        &store,
        player,
        &format!("ORDER {fence} 1 HOLD"),
    )
    .await?;
    let now = rig.world.world().world_ic_now().at;
    let poll = fire_flight(&mut rig.world, &store, now).await?;
    land_request(&mut rig.world, &store, &poll).await?;
    let slot = Slot::new(1)?;
    let saved = field_orders(&rig.world, organizer)?;
    let original = OrderBook::restore(&saved.book, &field_roster(&rig.world, organizer)?)?
        .delivered(slot)
        .ok_or("standing")?
        .order
        .sequence;
    commit(
        &mut rig.world,
        &store,
        player,
        &format!("ORDER {fence} 1 ADVANCE 4,5"),
    )
    .await?;
    commit(&mut rig.world, &store, organizer, "@unitgrant/revoke 2").await?;
    let now = rig.world.world().world_ic_now().at;
    let poll = fire_flight(&mut rig.world, &store, now).await?;
    land_request(&mut rig.world, &store, &poll).await?;
    let saved = field_orders(&rig.world, organizer)?;
    let roster = field_roster(&rig.world, organizer)?;
    let book = OrderBook::restore(&saved.book, &roster)?;
    assert!(
        book.pending(slot).is_none(),
        "transport arrival cannot restore revoked command rights"
    );
    assert_eq!(
        book.delivered(slot).ok_or("standing")?.order.sequence,
        original
    );
    commit(
        &mut rig.world,
        &store,
        organizer,
        &format!(
            "@unitgrant/issue #{}=#{} command_assigned_ai",
            unit.0, player.0
        ),
    )
    .await?;
    commit(
        &mut rig.world,
        &store,
        player,
        &format!("ORDER {fence} 1 ADVANCE 4,5 EXPIRES 1"),
    )
    .await?;
    let now = rig.world.world().world_ic_now().at;
    let poll = fire_flight(&mut rig.world, &store, now).await?;
    land_request(&mut rig.world, &store, &poll).await?;
    let saved = field_orders(&rig.world, organizer)?;
    let roster = field_roster(&rig.world, organizer)?;
    let book = OrderBook::restore(&saved.book, &roster)?;
    assert!(book.pending(slot).is_none());
    assert_eq!(
        book.standing(
            &roster,
            slot,
            u64::try_from(rig.world.world().world_ic_now().at)? * 1000
        )
        .map(|p| &p.objective),
        Some(&FieldObjective::Hold)
    );
    Ok(())
}

#[tokio::test]
async fn ic_field_leadership_uses_real_player_control_sessions_and_restart() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, cab) = field_fixture(&store).await?;
    let (organizer, player, carrier, unit) = (rig.organizer, rig.invitee, rig.carrier, rig.unit);
    rig.world.connect(player);
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=all").await?;
    let due = flight(&rig.world, unit, 1)?.due.at;
    let finish = fire_flight(&mut rig.world, &store, due).await?;
    land_request(&mut rig.world, &store, &finish).await?;
    let now_ic = rig.world.world().world_ic_now().at;
    let refresh = fire_flight(&mut rig.world, &store, now_ic).await?;
    land_request(&mut rig.world, &store, &refresh).await?;
    let selected = field_roster(&rig.world, organizer)?;
    assert_eq!(selected.leader().map(|l| l.character), Some(player));
    assert_ne!(
        selected.leader().map(|l| l.character),
        Some(organizer),
        "launching does not confer field command"
    );
    let now_ic = rig.world.world().world_ic_now().at;
    let unchanged = fire_flight(&mut rig.world, &store, now_ic).await;
    assert!(
        matches!(unchanged,Err(ref e) if e.to_string().contains("no flight completion queued")),
        "clock advance alone does not write another leadership record"
    );
    let now = rig.world.world().now();
    // Consume the session owner's disconnected/live-connection facts, without
    // inventing a separate DT grace timer.
    rig.world.world_mut().set_connected(player, false);
    rig.world.world_mut().set_live_conns(player, 0);
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert!(field_roster(&rig.world, organizer)?.leader().is_none());
    rig.world.world_mut().set_connected(player, true);
    rig.world.world_mut().set_live_conns(player, 1);
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert!(
        field_roster(&rig.world, organizer)?.leader().is_none(),
        "return hysteresis survives owner commits"
    );
    rig.world.set_now(now + 4);
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert_eq!(
        field_roster(&rig.world, organizer)?
            .leader()
            .map(|l| l.character),
        Some(player)
    );
    let request = submit(&mut rig.world, organizer, "+sortie/refresh 1:1")?;
    stage(&mut rig.world, &store, &request).await?;
    let crew = tacsim::port::ship::hull_view(rig.world.world(), carrier)
        .ok_or("hull")?
        .room(&mush_core::CanonicalId::new("crew")?)
        .ok_or("crew")?;
    let crew = mush_sim::anchored_space_of(rig.world.world(), crew).ok_or("space")?;
    // This player has ordinary room placement, not a Sim participant record.
    rig.world.world_mut().move_to(player, crew)?;
    assert!(
        rig.world.engine_mut().take_staged_action().is_err(),
        "leaving the actual console invalidates prepared leadership"
    );
    rig.world.world_mut().move_to(player, cab)?;
    persist_legacy(&mut rig.world, &store).await?;
    let term = field_roster(&rig.world, organizer)?.term();
    let now = rig.world.world().now();
    drop(rig);
    let mut rig = fixture(&store).await?;
    rig.world.set_now(now);
    assert_eq!(
        field_roster(&rig.world, organizer)?.term(),
        term,
        "restart preserves succession identity"
    );
    // A restarted player has neither a session nor a restored focus. Saved
    // leadership is history, never a replacement for either current fact.
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert!(field_roster(&rig.world, organizer)?.leader().is_none());
    Ok(())
}

#[tokio::test]
async fn ic_field_npc_commander_needs_current_contract_nomination_and_org_authority() -> TestResult
{
    let store = InMemoryStore::new();
    let (mut rig, _) = field_commander_fixture(&store, true, true).await?;
    let (organizer, npc, unit) = (rig.organizer, rig.invitee, rig.unit);
    let seating = submit(&mut rig.world, organizer, "+sortie/seat 1:1=1")?;
    persist_legacy(&mut rig.world, &store).await?;
    land_request(&mut rig.world, &store, &seating).await?;
    install_due_handoff(&mut rig.world, &store).await?;
    commit(&mut rig.world, &store, organizer, "+sortie/launch 1:1=1").await?;
    let due = flight(&rig.world, unit, 1)?.due.at;
    let finish = fire_flight(&mut rig.world, &store, due).await?;
    land_request(&mut rig.world, &store, &finish).await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert!(
        field_roster(&rig.world, organizer)?.leader().is_none(),
        "employment and nomination alone do not grant unit command"
    );
    commit(
        &mut rig.world,
        &store,
        organizer,
        &format!(
            "@unitgrant/issue #{}=#{} command_assigned_ai",
            unit.0, npc.0
        ),
    )
    .await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    let commanded = field_roster(&rig.world, organizer)?;
    assert_eq!(commanded.leader().map(|l| l.character), Some(npc));
    commit(&mut rig.world, &store, organizer, "@unitgrant/revoke 2").await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    let revoked = field_roster(&rig.world, organizer)?;
    assert!(revoked.leader().is_none());
    assert_eq!(revoked.term(), commanded.term() + 1);
    commit(
        &mut rig.world,
        &store,
        organizer,
        &format!(
            "@unitgrant/issue #{}=#{} command_assigned_ai",
            unit.0, npc.0
        ),
    )
    .await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert_eq!(
        field_roster(&rig.world, organizer)?
            .leader()
            .map(|l| l.character),
        Some(npc)
    );
    commit(&mut rig.world, &store, organizer, "+sortie/deactivate 1:1").await?;
    commit(&mut rig.world, &store, organizer, "+sortie/refresh 1:1").await?;
    assert!(field_roster(&rig.world, organizer)?.leader().is_none());
    let unresolved = flight(&rig.world, unit, 1)?;
    assert_eq!(unresolved.phase, tacsim::detachment::sortie::Phase::Field);
    assert!(
        unresolved.closed.is_none(),
        "disbanding does not recover the actual unit"
    );
    Ok(())
}
