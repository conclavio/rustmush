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
    fixture_keyed(store, false).await
}
async fn fixture_keyed(store: &InMemoryStore, geology: bool) -> TestResult<Rig> {
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
    // Production boot hydrates/seeds the immutable timeline before modules.
    // A rate-change action alone writes generation two, not its genesis.
    let configured = world.game_clock();
    mush_server::timeline::hydrate_world_timeline(&mut world, store, configured).await?;
    // Arm real periodic ticks on the same deterministic wall clock used below.
    world.set_now(100);
    let mut engine =
        Engine::with_modules(world, mush_server::world_task::base_registry(), modules)?;
    if geology {
        tacsim::surface::install_geology(
            &mut engine.world,
            mush_core::GeologyBoot::Key(mush_core::GeologySecret::new(1, [0x31; 32])?),
        );
    }
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

#[tokio::test]
async fn stock_surface_rigs_have_real_unoccupied_helm_consoles_across_restart() -> TestResult {
    let store = InMemoryStore::new();
    let rig = fixture(&store).await?;
    let port = tacsim::port::coop::programme_port(rig.world.world()).ok_or("programme port")?;
    let rigs = port
        .map
        .vehicles
        .as_ref()
        .ok_or("actual stock rigs")?
        .rigs
        .clone();
    assert!(!rigs.is_empty());
    let pilot = mush_sim::StationId::new("pilot").ok_or("station")?;
    let helm = mush_core::ChannelId::new("helm").ok_or("channel")?;
    let spatial = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("spatial")?;
    let mut generations = Vec::new();
    for row in &rigs {
        let unit = ObjId(row.tv);
        let cab = ObjId(
            *row.rooms
                .get(&mush_core::CanonicalId::new("cab")?)
                .ok_or("authored cab")?,
        );
        let station = spatial
            .stations
            .get(&unit)
            .and_then(|s| s.stations.get(&pilot))
            .ok_or("authored physical station")?;
        assert_eq!(station.space, Some(cab));
        assert!(station.channels.contains(&helm));
        assert!(
            matches!(spatial.spaces.get(&cab).map(|s|&s.anchor),Some(mush_sim::SpaceAnchor::MovingObject{host}) if *host==unit)
        );
        assert!(
            spatial
                .assignments
                .get(&unit)
                .is_none_or(|a| a.seats.is_empty()),
            "authoring the station never assigns an operator"
        );
        generations.push((unit, spatial.control_generation(unit)));
    }
    drop(rig);
    let restarted = fixture(&store).await?;
    let spatial = restarted
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("restored spatial")?;
    for (unit, generation) in generations {
        assert_eq!(
            spatial.control_generation(unit),
            generation,
            "reloading authored consoles must not churn control generations"
        );
        assert!(
            spatial
                .stations
                .get(&unit)
                .and_then(|s| s.stations.get(&pilot))
                .is_some()
        );
    }
    Ok(())
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
        duties: std::collections::BTreeSet::from([CrewDuty::Combat, CrewDuty::Mining]),
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

async fn mining_crew_fixture(store: &InMemoryStore) -> TestResult<(Rig, ObjId)> {
    mining_field_fixture(store, false).await
}
async fn rented_mining_fixture(store: &InMemoryStore) -> TestResult<(Rig, ObjId)> {
    let mut rig = fixture_keyed(store, true).await?;
    ordinary(&mut rig.world, GOD, "@tacsim/time rate=24")?;
    for _ in 0..40 {
        let status = text(&mut rig.world, GOD, "@tacsim/port status");
        let gated: Vec<_> = status
            .lines()
            .filter(|l| {
                l.ends_with(']')
                    && !l.starts_with("sky: ")
                    && !l.starts_with("geology: ")
                    && !(l.contains(" site: ")
                        && (l.contains("; opening, ")
                            || l.contains("; opening: its ground is still being mapped")))
            })
            .collect();
        if gated.len() > 1 && gated.iter().all(|l| l.ends_with("[ready]")) {
            break;
        }
        pulse(&mut rig.world, store, 6).await?;
    }
    *rig.world
        .world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .ok_or("account oracle")? = mush_sim::AccountOracle::new(Arc::new(PilotAccount {
        actor: rig.organizer,
        account: AccountId::mint("MiningCaptain", 1)?,
    }));
    let room = |world: &TestWorld, name: &str| -> TestResult<ObjId> {
        let key = mush_atlas::RecordKey::new(
            mush_core::CanonicalId::new("neighborhood.ceres_freeport")?,
            mush_core::CanonicalId::new(name)?,
        );
        tacsim::port::apply::room_by_key(world.world(), &key)
            .ok_or_else(|| format!("missing {name}").into())
    };
    let desk = room(&rig.world, "room.c05_registry")?;
    rig.world.world_mut().move_to(rig.organizer, desk)?;
    ordinary(&mut rig.world, rig.organizer, "+coop/enrol Field Moth")?;
    ordinary(&mut rig.world, rig.organizer, "+coop/refit prospector")?;
    ordinary(&mut rig.world, rig.organizer, "+licence/buy go")?;
    let counter = room(&rig.world, "room.c07_exchange")?;
    rig.world.world_mut().move_to(rig.organizer, counter)?;
    ordinary(&mut rig.world, rig.organizer, "+tv/lease go")?;
    let lease = rig
        .world
        .world()
        .module_state::<tacsim::TacsimStore>()
        .ok_or("tacsim")?
        .tv_leases_of(rig.organizer)
        .find(|l| l.live())
        .cloned()
        .ok_or("actual leased stock rig")?;
    rig.unit = lease.tv;
    rig.carrier = lease.hull;
    let cab = {
        let (_, row) =
            tacsim::port::vehicle::rig_row(rig.world.world(), rig.unit).ok_or("stock rig")?;
        ObjId(
            *row.rooms
                .get(&mush_core::CanonicalId::new("cab")?)
                .ok_or("cab")?,
        )
    };
    persist_legacy(&mut rig.world, store).await?;
    Ok((rig, cab))
}
async fn mining_field_fixture(store: &InMemoryStore, pair: bool) -> TestResult<(Rig, ObjId)> {
    let (mut rig, cab) = rented_mining_fixture(store).await?;
    commit(&mut rig.world, store, rig.organizer, "+tv/1/permissions go").await?;
    // Hire the actual shipped person, then lend through the owner's real loan.
    rig.invitee = rig
        .world
        .world()
        .module_state::<mush_content::ContentState>()
        .and_then(|s| {
            s.latches.applied(
                &mush_core::CanonicalId::new(tacsim::detachment::crew::content::PACK).ok()?,
            )
        })
        .and_then(|l| {
            l.entries()
                .get(&mush_core::CanonicalId::new("crew.ilya_mendes").ok()?)
        })
        .and_then(mush_content::AppliedEntry::object)
        .ok_or("shipped mining crew")?;
    let terms: tacsim::detachment::crew::HireTerms = serde_json::from_value(
        rig.world
            .world()
            .attr(rig.invitee, tacsim::detachment::crew::OFFER_ATTR)
            .and_then(|a| a.as_json())
            .ok_or("authored crew terms")?
            .clone(),
    )?;
    let room = rig
        .world
        .world()
        .get(rig.invitee)
        .and_then(|o| o.location)
        .ok_or("crew bar")?;
    rig.world.world_mut().move_to(rig.organizer, room)?;
    persist_legacy(&mut rig.world, store).await?;
    commit(
        &mut rig.world,
        store,
        rig.organizer,
        &format!(
            "+sortie/hire #{}={}",
            rig.invitee.0,
            terms.digest()?.to_hex()
        ),
    )
    .await?;
    ordinary(
        &mut rig.world,
        rig.organizer,
        &format!("+sortie/board #{}", rig.invitee.0),
    )?;
    persist_legacy(&mut rig.world, store).await?;
    let loan = {
        let h = rig
            .world
            .world()
            .module_state::<mush_org::unit_authority::runtime::UnitAuthorityHandles>()
            .ok_or("grants")?;
        rig.world
            .engine()
            .action_state()
            .get(&h.registry, &())?
            .ok_or("owner loans")?
            .grants
            .values()
            .find(|g| g.capability == mush_org::unit_authority::UnitCapability::OperateCrew)
            .ok_or("operate loan")?
            .id
            .0
    };
    commit(
        &mut rig.world,
        store,
        rig.organizer,
        &format!(
            "@unitgrant/issue #{}=#{} operate_crew parent={loan}",
            rig.unit.0, rig.invitee.0
        ),
    )
    .await?;
    let site = rig
        .world
        .world()
        .module_state::<tacsim::TacsimStore>()
        .ok_or("tacsim")?
        .port_maps()
        .find(|m| m.entry.as_str() == "ahuna_port")
        .and_then(|m| m.domain)
        .ok_or("Ahuna domain")?;
    let mut opened = false;
    for _ in 0..80 {
        let status = text(&mut rig.world, GOD, "@tacsim/port status");
        if status
            .lines()
            .any(|l| l.contains("ahuna_port site:") && l.ends_with("[ready]"))
        {
            opened = true;
            break;
        }
        pulse(&mut rig.world, store, 6).await?;
    }
    assert!(
        opened,
        "the real site opener must finish before the carrier arrives"
    );
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/place/domain #{}={site}", rig.carrier.0),
    )?;
    let crew_room = {
        let port = tacsim::port::coop::programme_port(rig.world.world()).ok_or("programme")?;
        let row = port
            .map
            .hulls
            .iter()
            .find(|r| r.hull == rig.carrier.0)
            .ok_or("charter hull")?;
        ObjId(
            *row.rooms
                .get(&mush_core::CanonicalId::new("crew")?)
                .ok_or("crew")?,
        )
    };
    rig.world.world_mut().move_to(rig.organizer, crew_room)?;
    let leader = if pair {
        let unit = rig
            .world
            .world()
            .iter()
            .find(|o| o.name == "Hidden preferred unit name")
            .ok_or("leader unit")?
            .id;
        let cab = rig
            .world
            .world_mut()
            .create(ObjKind::Room, "Leader field cab", GOD, None)?;
        for (id, behavior) in [(unit, "sim.participant"), (cab, "sim.interior")] {
            rig.world.world_mut().attach_behavior(
                id,
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
            &format!("@unit/create #{}=tacsim.unit.line_tank", unit.0),
        )?;
        persist_legacy(&mut rig.world, store).await?;
        commit(
            &mut rig.world,
            store,
            GOD,
            &format!("@spatial/space #{}=moving:#{}", cab.0, unit.0),
        )
        .await?;
        let (bay, slot) = {
            let view =
                tacsim::port::ship::hull_view(rig.world.world(), rig.carrier).ok_or("hull")?;
            let (b, r) = view.bay().ok_or("bay")?;
            (r, b.slot.id.to_string())
        };
        for cmd in [
            format!("@spatial/slot #{}=berths,1", cab.0),
            format!("@spatial/place/interior #{}=#{},{}", unit.0, bay.0, slot),
            format!(
                "@spatial/station #{}=pilot,pilot,channels:helm,space:#{}",
                unit.0, cab.0
            ),
            format!("@spatial/assign #{}=pilot,#{}", unit.0, rig.organizer.0),
            format!("@asset/title #{}=#{}", unit.0, rig.organizer.0),
        ] {
            ordinary(&mut rig.world, GOD, &cmd)?;
        }
        rig.world.world_mut().move_to(rig.organizer, cab)?;
        rig.world.connect(rig.organizer);
        ordinary(
            &mut rig.world,
            rig.organizer,
            &format!("+focus #{}/pilot", unit.0),
        )?;
        persist_legacy(&mut rig.world, store).await?;
        for cap in ["operate_crew", "command_assigned_ai"] {
            commit(
                &mut rig.world,
                store,
                rig.organizer,
                &format!("@unitgrant/issue #{}=#{} {cap}", unit.0, rig.organizer.0),
            )
            .await?;
        }
        Some(unit)
    } else {
        None
    };
    persist_legacy(&mut rig.world, store).await?;
    let mut payload = plan(&rig);
    payload["slots"][0]["controller"]["kind"] = json!("npc");
    if let Some(unit) = leader {
        payload["slots"].as_array_mut().ok_or("slots")?.push(json!({"slot":2,"unit":unit.0,"controller":{"kind":"player","character":rig.organizer.0},"role":"scout","mind":null}));
    }

    commit(
        &mut rig.world,
        store,
        rig.organizer,
        &format!("+sortie/draft {payload}"),
    )
    .await?;
    if let Some(unit) = leader {
        let selected = draft(&rig.world, rig.organizer, 1)?;
        commit(
            &mut rig.world,
            store,
            rig.organizer,
            &declaration("accept", &selected),
        )
        .await?;
        commit(
            &mut rig.world,
            store,
            rig.organizer,
            &format!("@station/handoff #{}/pilot", unit.0),
        )
        .await?;
        install_due_handoff(&mut rig.world, store).await?;
        commit(
            &mut rig.world,
            store,
            rig.organizer,
            &declaration("ready", &selected),
        )
        .await?;
    }
    commit(
        &mut rig.world,
        store,
        rig.organizer,
        "+sortie/activate 1:1=mining",
    )
    .await?;
    let seating = submit(&mut rig.world, rig.organizer, "+sortie/seat 1:1=1")?;
    persist_legacy(&mut rig.world, store).await?;
    land_request(&mut rig.world, store, &seating).await?;
    install_due_handoff(&mut rig.world, store).await?;
    commit(&mut rig.world, store, rig.organizer, "+sortie/launch 1:1=1").await?;
    let departure = flight(&rig.world, rig.unit, 1)?;
    let finish = fire_flight(&mut rig.world, store, departure.due.at).await?;
    land_request(&mut rig.world, store, &finish).await?;
    if let Some(unit) = leader {
        commit(&mut rig.world, store, rig.organizer, "+sortie/launch 1:1=2").await?;
        let due = flight(&rig.world, unit, 1)?.due.at;
        for _ in 0..5 {
            if flight(&rig.world, unit, 1)?.phase == tacsim::detachment::sortie::Phase::Field {
                break;
            }
            let finish = fire_flight(&mut rig.world, store, due).await?;
            land_request(&mut rig.world, store, &finish).await?;
        }
        commit(&mut rig.world, store, rig.organizer, "+sortie/refresh 1:1").await?;
        assert_eq!(
            field_roster(&rig.world, rig.organizer)?
                .leader()
                .map(|l| l.character),
            Some(rig.organizer)
        );
    }
    Ok((rig, cab))
}

#[tokio::test]
async fn hired_mining_operator_uses_real_stock_rig_and_existing_surface_host() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, cab) = mining_crew_fixture(&store).await?;
    assert_eq!(
        rig.world.world().get(rig.invitee).and_then(|o| o.location),
        Some(cab)
    );
    assert!(actual_helm(&rig.world, rig.unit, rig.invitee)?);
    let before = flight(&rig.world, rig.unit, 1)?;
    let line = text(&mut rig.world, rig.invitee, "+tv/1 scan");
    assert!(!line.contains("no charter"), "{line}");
    commit(&mut rig.world, &store, rig.invitee, "+tv/1 scan go").await?;
    let h = rig
        .world
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .ok_or("ports")?;
    let record = rig
        .world
        .engine()
        .action_state()
        .get(
            &h.families.excursions,
            &tacsim::surface::excursion::RigKey(rig.unit),
        )?
        .ok_or("real surface work")?;
    assert_eq!(
        record
            .field_crew
            .as_ref()
            .ok_or("physical crew link")?
            .operator,
        rig.invitee
    );
    assert_eq!(
        record.excursion.as_ref().ok_or("excursion")?.operator,
        rig.invitee
    );
    assert!(record.sortie.is_none(), "DT retains the only epoch");
    assert_eq!(flight(&rig.world, rig.unit, 1)?, before);
    settle_surface(&mut rig, &store).await?;
    let record = surface_record(&rig.world, rig.unit)?.ok_or("surface record")?;
    assert_eq!(
        record.last.as_ref().ok_or("actual scan outcome")?.result,
        tacsim::surface::excursion::ActResult::Done
    );
    let drive = adjacent_drive(&mut rig, &store).await?;
    persist_legacy(&mut rig.world, &store).await?;
    land_request(&mut rig.world, &store, &drive).await?;
    settle_surface(&mut rig, &store).await?;
    assert_eq!(
        surface_record(&rig.world, rig.unit)?
            .ok_or("surface record")?
            .last
            .ok_or("drive outcome")?
            .result,
        tacsim::surface::excursion::ActResult::Done
    );
    commit(&mut rig.world, &store, rig.invitee, "+tv/1 sound go").await?;
    settle_surface(&mut rig, &store).await?;
    // Choose only from this NPC's own completed sounding, never a staff geology peek.
    let site = mush_core::CanonicalId::new("ahuna_port")?;
    let chart = {
        let h = rig
            .world
            .world()
            .module_state::<tacsim::port::PortHandles>()
            .ok_or("ports")?;
        rig.world
            .engine()
            .action_state()
            .get(
                &h.families.charts,
                &tacsim::surface::chart::ChartKey {
                    principal: rig.invitee,
                    site: site.clone(),
                },
            )?
            .cloned()
            .ok_or("NPC's own chart")?
    };
    let mut pockets = Vec::new();
    let stand = chart.sound_stand.ok_or("sounding stand")?;
    for y in 0..64 {
        for x in 0..64 {
            if !chart.pocket_sounded((x, y)) {
                continue;
            }
            let reading = tacsim::surface::chart::pocket_reading(
                rig.world.world(),
                &site,
                Some(&chart),
                (x, y),
            );
            if ["lean", "fair", "rich"]
                .iter()
                .any(|word| reading.contains(word))
            {
                pockets.push((
                    (x * 4 + 2 - stand[0]).abs() + (y * 4 + 2 - stand[1]).abs(),
                    x,
                    y,
                ));
            }
        }
    }
    pockets.sort();
    let mut moved = false;
    for (_, x, y) in pockets {
        let address = tacsim::surface::address::pocket_address(x, y).ok_or("pocket")?;
        text(
            &mut rig.world,
            rig.invitee,
            &format!("+tv/1 drive {address} go"),
        );
        if let Some(request) = rig.world.engine_mut().take_authoritative_submit() {
            persist_legacy(&mut rig.world, &store).await?;
            land_request(&mut rig.world, &store, &request).await?;
            settle_surface(&mut rig, &store).await?;
            moved = true;
            break;
        }
    }
    assert!(moved, "the own sounding must offer reachable ore");
    commit(&mut rig.world, &store, rig.invitee, "+tv/1 mine 1 go").await?;
    let run = surface_record(&rig.world, rig.unit)?
        .ok_or("surface")?
        .run
        .ok_or("run")?;
    let inventory = rig
        .world
        .world()
        .module_state::<mush_inventory::InventoryHandles>()
        .ok_or("inventory")?;
    let key = mush_inventory::pool_key(
        mush_inventory::EndpointRef(mush_economy::VaultId(run.source.ok_or("source")?)),
        &run.good,
    )?;
    let source_before = pool_amount(
        &rig.world,
        mush_economy::VaultId(run.source.ok_or("source")?),
        &run.good,
    )?;
    assert!(
        rig.world
            .engine()
            .action_state()
            .get(&inventory.0.pools, &key)?
            .is_some(),
        "the site opener must supply the real deposit inventory"
    );
    settle_surface(&mut rig, &store).await?;
    let mining_outcome = surface_record(&rig.world, rig.unit)?
        .ok_or("surface record")?
        .last
        .ok_or("mining outcome")?;
    assert_eq!(
        mining_outcome.result,
        tacsim::surface::excursion::ActResult::Done,
        "{mining_outcome:?}"
    );
    let mined = surface_record(&rig.world, rig.unit)?
        .ok_or("surface record")?
        .last
        .ok_or("actual cycle")?
        .mined
        .ok_or("mining receipt")?;
    assert!(
        mined.tonnes > 0,
        "real ore must move from the site into this rig's hopper: {mined:?}"
    );
    assert_eq!(
        pool_amount(
            &rig.world,
            mush_economy::VaultId(run.source.ok_or("source")?),
            &run.good
        )?,
        source_before - i64::from(mined.tonnes)
    );
    assert_eq!(
        pool_amount(&rig.world, mush_economy::VaultId(run.hopper), &run.good)?,
        i64::from(mined.tonnes)
    );
    commit(&mut rig.world, &store, rig.invitee, "+tv/1 recover go").await?;
    settle_surface(&mut rig, &store).await?;
    assert_eq!(
        surface_record(&rig.world, rig.unit)?
            .ok_or("surface record")?
            .last
            .ok_or("recover outcome")?
            .result,
        tacsim::surface::excursion::ActResult::Done
    );
    for _ in 0..4 {
        if flight(&rig.world, rig.unit, 1)?.phase == tacsim::detachment::sortie::Phase::Returned {
            break;
        }
        pulse(&mut rig.world, &store, 6).await?;
    }
    assert_eq!(
        flight(&rig.world, rig.unit, 1)?.phase,
        tacsim::detachment::sortie::Phase::Returned
    );
    let record = surface_record(&rig.world, rig.unit)?.ok_or("surface record")?;
    assert_eq!(
        record.last.as_ref().ok_or("unload receipt")?.kind,
        tacsim::surface::excursion::ActKind::Unload,
        "a loaded recovery must finish the real unload"
    );
    assert!(record.excursion.is_none() && record.sortie.is_none() && record.charge.in_bay);
    let hold = {
        let view =
            tacsim::port::ship::hull_view(rig.world.world(), rig.carrier).ok_or("carrier")?;
        tacsim::port::ship::hold_vault(rig.world.world(), &view).ok_or("hold")?
    };
    assert_eq!(
        pool_amount(&rig.world, hold, &run.good)?,
        i64::from(mined.tonnes)
    );
    assert_eq!(
        pool_amount(&rig.world, mush_economy::VaultId(run.hopper), &run.good)?,
        0
    );
    commit(
        &mut rig.world,
        &store,
        rig.organizer,
        "+sortie/launch 1:1=1",
    )
    .await?;
    assert_eq!(
        flight(&rig.world, rig.unit, 2)?.phase,
        tacsim::detachment::sortie::Phase::Departing
    );
    Ok(())
}

fn surface_record(
    world: &TestWorld,
    unit: ObjId,
) -> TestResult<Option<tacsim::surface::excursion::RigRecord>> {
    let h = world
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .ok_or("ports")?;
    Ok(world
        .engine()
        .action_state()
        .get(
            &h.families.excursions,
            &tacsim::surface::excursion::RigKey(unit),
        )?
        .cloned())
}

fn pool_amount(
    world: &TestWorld,
    vault: mush_economy::VaultId,
    good: &mush_core::CanonicalId,
) -> TestResult<i64> {
    let inventory = world
        .world()
        .module_state::<mush_inventory::InventoryHandles>()
        .ok_or("inventory")?;
    let key = mush_inventory::pool_key(mush_inventory::EndpointRef(vault), good)?;
    Ok(world
        .engine()
        .action_state()
        .get(&inventory.0.pools, &key)?
        .map_or(0, |s| s.current()))
}

async fn adjacent_drive(
    rig: &mut Rig,
    store: &InMemoryStore,
) -> TestResult<Box<mush_core::AuthoritativeSubmit>> {
    let mut diagnostics = Vec::new();
    for bearing in ["n", "ne", "e", "se", "s", "sw", "w", "nw"]
        .into_iter()
        .cycle()
        .take(16)
    {
        diagnostics.push(text(
            &mut rig.world,
            rig.invitee,
            &format!("+tv/1 drive {bearing} 20 go"),
        ));
        if let Some(request) = rig.world.engine_mut().take_authoritative_submit() {
            return Ok(request);
        }
        pulse(&mut rig.world, store, 6).await?;
    }
    Err(format!("no drivable adjacent ground: {diagnostics:?}").into())
}

#[tokio::test]
async fn physical_surface_completed_movement_still_spends_charge_after_station_loss() -> TestResult
{
    let store = InMemoryStore::new();
    let (mut rig, _) = mining_crew_fixture(&store).await?;
    let start = adjacent_drive(&mut rig, &store).await?;
    persist_legacy(&mut rig.world, &store).await?;
    land_request(&mut rig.world, &store, &start).await?;
    let before = surface_record(&rig.world, rig.unit)?.ok_or("surface record")?;
    let act = before.act.as_ref().ok_or("drive")?;
    assert!(act.reserved_cu > 0);
    let rate = i64::from(rig.world.world().world_timeline().current().rate.max(1));
    let ahead = (act.due_ic - rig.world.world().world_ic_now().at).max(0);
    let now = rig.world.world().now() + ((ahead + rate - 1) / rate).max(6);
    rig.world.set_now(now);
    rig.world.fire_due(now * 1000);
    let mut finish = None;
    while let Some(request) = rig.world.engine_mut().take_authoritative_submit() {
        if request.action.key().id.as_str() == tacsim::surface::act::SURFACE_DONE_ACTION
            && request.actor == rig.invitee
        {
            finish = Some(request);
        }
    }
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/unassign #{}=pilot", rig.unit.0),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    let finish = finish.ok_or("actual drive finish")?;
    land_request(&mut rig.world, &store, &finish).await?;
    let after = surface_record(&rig.world, rig.unit)?.ok_or("surface record")?;
    assert_eq!(
        after.last.ok_or("actual outcome")?.result,
        tacsim::surface::excursion::ActResult::Done
    );
    assert_eq!(
        after.charge.value_cu,
        before.charge.value_cu - act.reserved_cu,
        "a later control change cannot refund an actual drive"
    );
    Ok(())
}

#[tokio::test]
async fn physical_surface_final_take_rechecks_current_asset_permission() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _) = mining_crew_fixture(&store).await?;
    let request = submit(&mut rig.world, rig.invitee, "+tv/1 scan go")?;
    stage(&mut rig.world, &store, &request).await?;
    let org = {
        let economy = rig
            .world
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .ok_or("economy")?;
        let mush_org::PrincipalId::Organization(org) = economy
            .snapshot()
            .asset(rig.unit)
            .ok_or("title")?
            .title_owner
        else {
            return Err("lessor".into());
        };
        org
    };
    ordinary(&mut rig.world, GOD, &format!("@org/status {org}=suspended"))?;
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    assert!(surface_record(&rig.world, rig.unit)?.is_none());
    assert_eq!(
        flight(&rig.world, rig.unit, 1)?.phase,
        tacsim::detachment::sortie::Phase::Field
    );
    Ok(())
}

#[tokio::test]
async fn physical_field_idle_wait_does_not_recall_the_crew() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _) = mining_crew_fixture(&store).await?;
    commit(&mut rig.world, &store, rig.invitee, "+tv/1 scan go").await?;
    settle_surface(&mut rig, &store).await?;
    let before = surface_record(&rig.world, rig.unit)?.ok_or("surface record")?;
    let rate = i64::from(rig.world.world().world_timeline().current().rate.max(1));
    pulse(&mut rig.world, &store, 2 * 86_400 / rate + 1).await?;
    let after = surface_record(&rig.world, rig.unit)?.ok_or("surface record")?;
    assert_eq!(
        after.ordinal, before.ordinal,
        "idle recall cannot issue a new physical control action"
    );
    assert!(after.act.is_none() && after.run.is_none() && after.excursion.is_some());
    assert_eq!(
        flight(&rig.world, rig.unit, 1)?.phase,
        tacsim::detachment::sortie::Phase::Field
    );
    Ok(())
}

#[tokio::test]
async fn physical_surface_station_loss_finishes_without_a_reading_or_charge_spend() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _) = mining_crew_fixture(&store).await?;
    commit(&mut rig.world, &store, rig.invitee, "+tv/1 scan go").await?;
    let before = surface_record(&rig.world, rig.unit)?.ok_or("surface record")?;
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/unassign #{}=pilot", rig.unit.0),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    settle_surface(&mut rig, &store).await?;
    let after = surface_record(&rig.world, rig.unit)?.ok_or("surface record")?;
    assert_eq!(after.charge, before.charge);
    assert_eq!(
        after.last.ok_or("actual refusal")?.result,
        tacsim::surface::excursion::ActResult::Refused {
            code: tacsim::surface::excursion::RefusedCode::Unworked
        }
    );
    let h = rig
        .world
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .ok_or("ports")?;
    assert!(
        rig.world
            .engine()
            .action_state()
            .get(
                &h.families.charts,
                &tacsim::surface::chart::ChartKey {
                    principal: rig.invitee,
                    site: before.excursion.ok_or("excursion")?.site
                }
            )?
            .is_none()
    );
    assert_eq!(
        flight(&rig.world, rig.unit, 1)?.phase,
        tacsim::detachment::sortie::Phase::Field
    );
    Ok(())
}
async fn settle_surface(rig: &mut Rig, store: &InMemoryStore) -> TestResult {
    for _ in 0..16 {
        let record = surface_record(&rig.world, rig.unit)?.ok_or("surface work")?;
        let due = record
            .act
            .as_ref()
            .map(|a| a.due_ic)
            .into_iter()
            .chain(record.run.as_ref().map(|r| r.due_ic))
            .min();
        let Some(due) = due else {
            if record.queued.is_some() {
                pulse(&mut rig.world, store, 6).await?;
                continue;
            }
            return Ok(());
        };
        let rate = i64::from(rig.world.world().world_timeline().current().rate.max(1));
        let ahead = (due - rig.world.world().world_ic_now().at).max(0);
        let now = rig.world.world().now() + ((ahead + rate - 1) / rate).max(6);
        rig.world.set_now(now);
        rig.world.fire_due(now * 1000);
        persist_legacy(&mut rig.world, store).await?;
        while let Some(request) = rig.world.engine_mut().take_authoritative_submit() {
            land_request(&mut rig.world, store, &request).await?;
        }
    }
    Err("surface work did not settle".into())
}

async fn pulse(world: &mut TestWorld, store: &InMemoryStore, seconds: i64) -> TestResult {
    // Keep background site opening bounded to this story's Ahuna site. Other
    // clerks verify by pull and retry later; every landed act uses the coordinator.
    let other_sites: std::collections::BTreeSet<_> = world
        .world()
        .module_state::<tacsim::TacsimStore>()
        .ok_or("tacsim")?
        .port_maps()
        .filter(|m| m.entry.as_str() != "ahuna_port")
        .filter_map(|m| m.site.as_ref().and_then(|s| s.clerk).map(ObjId))
        .collect();
    let now = world.world().now() + seconds;
    world.set_now(now);
    world.fire_due(now * 1000);
    persist_legacy(world, store).await?;
    while let Some(request) = world.engine_mut().take_authoritative_submit() {
        if other_sites.contains(&request.actor) {
            continue;
        }
        land_request(world, store, &request).await?;
    }
    Ok(())
}

#[tokio::test]
async fn stock_cab_console_is_added_to_an_existing_rig_on_restart() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let unit = ObjId(
        tacsim::port::coop::programme_port(rig.world.world())
            .ok_or("programme")?
            .map
            .vehicles
            .as_ref()
            .ok_or("vehicles")?
            .rigs
            .first()
            .ok_or("rig")?
            .tv,
    );
    let parent = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("spatial")?
        .parent_of(unit);
    ordinary(
        &mut rig.world,
        GOD,
        &format!("@spatial/unstation #{}=pilot", unit.0),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    drop(rig);
    let rig = fixture(&store).await?;
    let spatial = rig
        .world
        .world()
        .module_state::<mush_sim::SpatialState>()
        .ok_or("spatial")?;
    let pilot = mush_sim::StationId::new("pilot").ok_or("pilot")?;
    assert!(
        spatial
            .stations
            .get(&unit)
            .is_some_and(|s| s.stations.contains_key(&pilot))
    );
    assert_eq!(
        spatial.parent_of(unit),
        parent,
        "authoring a missing console does not move the rig"
    );
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

#[tokio::test]
async fn ic_field_npc_mines_and_returns_from_a_real_player_leaders_packet() -> TestResult {
    use mush_tactics::detachment::{OrderBook, Slot};
    let store = InMemoryStore::new();
    let (mut rig, _) = mining_field_fixture(&store, true).await?;
    let roster = field_roster(&rig.world, rig.organizer)?;
    let command = format!(
        "ORDER DT:{}:{}:{} 1 MINE",
        roster.id().0,
        roster.revision(),
        roster.term()
    );
    commit(&mut rig.world, &store, rig.organizer, &command).await?;
    let slot = Slot::new(1)?;
    let orders = field_orders(&rig.world, rig.organizer)?;
    assert!(
        OrderBook::restore(&orders.book, &roster)?
            .delivered(slot)
            .is_none()
    );
    let mut premature = submit(&mut rig.world, rig.invitee, "+tv/1 scan go")?;
    premature.parameters["field_order"] = json!("1");
    assert!(
        stage(&mut rig.world, &store, &premature).await.is_err(),
        "a queued message cannot authorize an attempt"
    );
    assert!(surface_record(&rig.world, rig.unit)?.is_none());
    let mut restarted = false;
    for _ in 0..300 {
        pulse(&mut rig.world, &store, 1).await?;
        if !restarted && surface_record(&rig.world, rig.unit)?.is_some_and(|r| r.run.is_some()) {
            persist_legacy(&mut rig.world, &store).await?;
            let now = rig.world.world().now();
            let loaded = fixture_keyed(&store, true).await?;
            rig.world = loaded.world;
            rig.world.set_now(now);
            assert_eq!(rig.world.world().world_timeline().current().rate, 24);
            *rig.world
                .world_mut()
                .module_state_mut::<mush_sim::AccountOracle>()
                .ok_or("oracle")? = mush_sim::AccountOracle::new(Arc::new(PilotAccount {
                actor: rig.organizer,
                account: AccountId::mint("MiningCaptain", 1)?,
            }));
            restarted = true;
        }
        let orders = field_orders(&rig.world, rig.organizer)?;
        let roster = field_roster(&rig.world, rig.organizer)?;
        let book = OrderBook::restore(&orders.book, &roster)?;
        if let Some(d) = book.delivered(slot) {
            assert!(
                !d.acknowledgement
                    .is_some_and(|a| a.response == mush_tactics::detachment::Response::Negative),
                "task refused: {:?}; record {:?}",
                d,
                surface_record(&rig.world, rig.unit)?
            );
            if d.completed_at_ms.is_some() {
                let record = surface_record(&rig.world, rig.unit)?.ok_or("actual surface work")?;
                let last = record.last.ok_or("actual unload")?;
                assert_eq!(last.kind, tacsim::surface::excursion::ActKind::Unload);
                assert!(last.moved.values().any(|n| *n > 0));
                assert!(d.attempted_at_ms.is_some());
                assert_eq!(
                    flight(&rig.world, rig.unit, 1)?.phase,
                    tacsim::detachment::sortie::Phase::Returned
                );
                assert!(restarted, "the actual mining cycle must cross a boot");
                let report = text(&mut rig.world, rig.organizer, "+sortie/orders 1:1");
                assert!(report.contains("Execution unconfirmed."), "{report}");
                assert!(
                    !report.contains("completed at"),
                    "private work must not become instantaneous issuer telemetry"
                );
                return Ok(());
            }
        }
    }
    Err(format!(
        "NPC did not finish: {:?}; record {:?}",
        field_orders(&rig.world, rig.organizer)?,
        surface_record(&rig.world, rig.unit)?
    )
    .into())
}

async fn issue_npc_task(rig: &mut Rig, store: &InMemoryStore, words: &str) -> TestResult {
    let roster = field_roster(&rig.world, rig.organizer)?;
    commit(
        &mut rig.world,
        store,
        rig.organizer,
        &format!(
            "ORDER DT:{}:{}:{} 1 {words}",
            roster.id().0,
            roster.revision(),
            roster.term()
        ),
    )
    .await
}
#[tokio::test]
async fn ic_field_npc_hold_replacement_stops_old_mining_without_completing_hold() -> TestResult {
    use mush_tactics::detachment::{OrderBook, Slot};
    let store = InMemoryStore::new();
    let (mut rig, _) = mining_field_fixture(&store, true).await?;
    issue_npc_task(&mut rig, &store, "MINE").await?;
    let mut running = false;
    for _ in 0..200 {
        pulse(&mut rig.world, &store, 1).await?;
        if surface_record(&rig.world, rig.unit)?.is_some_and(|r| r.run.is_some()) {
            running = true;
            break;
        }
    }
    assert!(running, "actual source run must start");
    let old = surface_record(&rig.world, rig.unit)?
        .ok_or("record")?
        .field_work
        .ok_or("work")?
        .sequence;
    issue_npc_task(&mut rig, &store, "HOLD").await?;
    let mut holding = false;
    for _ in 0..100 {
        pulse(&mut rig.world, &store, 1).await?;
        let orders = field_orders(&rig.world, rig.organizer)?;
        let roster = field_roster(&rig.world, rig.organizer)?;
        let book = OrderBook::restore(&orders.book, &roster)?;
        let d = book.delivered(Slot::new(1)?).ok_or("delivered")?;
        if d.order.sequence > old
            && surface_record(&rig.world, rig.unit)?
                .is_some_and(|r| r.run.is_none() && r.act.is_none())
        {
            assert!(
                matches!(&d.order.directive,mush_tactics::detachment::Directive::Objective(p) if p.objective==tacsim::detachment::sortie::FieldObjective::Hold)
            );
            assert!(
                d.completed_at_ms.is_none(),
                "old extraction never completes HOLD"
            );
            holding = true;
            break;
        }
    }
    assert!(holding, "replacement must stop the old run");
    let before = surface_record(&rig.world, rig.unit)?.ok_or("record")?;
    pulse(&mut rig.world, &store, 12).await?;
    assert_eq!(
        surface_record(&rig.world, rig.unit)?
            .ok_or("record")?
            .ordinal,
        before.ordinal,
        "HOLD cannot restart mining or recall the rig"
    );
    Ok(())
}
#[tokio::test]
async fn ic_field_npc_unsupported_and_expired_orders_cannot_start_work() -> TestResult {
    use mush_tactics::detachment::{OrderBook, Response, Slot};
    let store = InMemoryStore::new();
    let (mut rig, _) = mining_field_fixture(&store, true).await?;
    issue_npc_task(&mut rig, &store, "SCREEN 90").await?;
    let slot = Slot::new(1)?;
    let mut refused = false;
    for _ in 0..15 {
        pulse(&mut rig.world, &store, 1).await?;
        let orders = field_orders(&rig.world, rig.organizer)?;
        let roster = field_roster(&rig.world, rig.organizer)?;
        let book = OrderBook::restore(&orders.book, &roster)?;
        if let Some(d) = book.delivered(slot) {
            assert!(d.attempted_at_ms.is_none() && d.completed_at_ms.is_none());
            if d.acknowledgement
                .is_some_and(|a| a.response == Response::Negative)
            {
                refused = true;
                break;
            }
        }
    }
    assert!(refused, "unsupported work must send a real NEGATIVE");
    issue_npc_task(&mut rig, &store, "MINE EXPIRES 1").await?;
    pulse(&mut rig.world, &store, 12).await?;
    assert!(surface_record(&rig.world, rig.unit)?.is_none());
    let orders = field_orders(&rig.world, rig.organizer)?;
    let roster = field_roster(&rig.world, rig.organizer)?;
    let book = OrderBook::restore(&orders.book, &roster)?;
    assert!(book.pending(slot).is_none());
    Ok(())
}

#[tokio::test]
async fn ic_field_npc_advance_and_withdraw_complete_only_from_actual_movement() -> TestResult {
    use mush_tactics::detachment::{OrderBook, Slot};
    let store = InMemoryStore::new();
    let (mut rig, _) = mining_field_fixture(&store, true).await?;
    // Ask the existing route planner for a legal adjacent cell; discard the
    // unadmitted hand command. Only the subsequently delivered task may move it.
    let planned = adjacent_drive(&mut rig, &store).await?;
    let number = planned.parameters["to"].as_u64().ok_or("actual target")?;
    let target = [i64::try_from(number % 256)?, i64::try_from(number / 256)?];
    issue_npc_task(
        &mut rig,
        &store,
        &format!("ADVANCE {},{}", target[0], target[1]),
    )
    .await?;
    let slot = Slot::new(1)?;
    let mut arrived = false;
    for _ in 0..70 {
        pulse(&mut rig.world, &store, 1).await?;
        let orders = field_orders(&rig.world, rig.organizer)?;
        let roster = field_roster(&rig.world, rig.organizer)?;
        let book = OrderBook::restore(&orders.book, &roster)?;
        if let Some(d) = book.delivered(slot)
            && d.completed_at_ms.is_some()
        {
            assert!(d.attempted_at_ms.is_some());
            assert_eq!(
                surface_record(&rig.world, rig.unit)?
                    .ok_or("record")?
                    .last
                    .ok_or("actual arrival")?
                    .arrived,
                Some(target)
            );
            arrived = true;
            break;
        }
    }
    assert!(arrived, "ADVANCE must finish the real drive");
    issue_npc_task(&mut rig, &store, "WITHDRAW").await?;
    for _ in 0..70 {
        pulse(&mut rig.world, &store, 1).await?;
        let orders = field_orders(&rig.world, rig.organizer)?;
        let roster = field_roster(&rig.world, rig.organizer)?;
        let book = OrderBook::restore(&orders.book, &roster)?;
        if book
            .delivered(slot)
            .is_some_and(|d| d.order.sequence == 2 && d.completed_at_ms.is_some())
            && flight(&rig.world, rig.unit, 1)?.phase == tacsim::detachment::sortie::Phase::Returned
        {
            assert_eq!(
                surface_record(&rig.world, rig.unit)?
                    .ok_or("record")?
                    .last
                    .ok_or("actual recover")?
                    .kind,
                tacsim::surface::excursion::ActKind::Recover
            );
            return Ok(());
        }
    }
    Err("WITHDRAW must physically return before completion".into())
}

#[tokio::test]
async fn rental_operational_rights_are_four_real_owner_loans_in_one_coordinator_commit()
-> TestResult {
    use mush_org::unit_authority::runtime::UnitAuthorityHandles;
    use mush_org::unit_authority::{UnitCapability, UnitGrantParent, UnitGrantee};
    let store = InMemoryStore::new();
    let (mut rig, _) = rented_mining_fixture(&store).await?;
    let h = rig
        .world
        .world()
        .module_state::<UnitAuthorityHandles>()
        .ok_or("unit grants")?
        .clone();
    assert!(
        rig.world
            .engine()
            .action_state()
            .get(&h.registry, &())?
            .is_none()
    );
    let preview = text(&mut rig.world, rig.organizer, "+tv/1/permissions");
    assert!(preview.contains("operate/crew and command"), "{preview}");
    assert!(rig.world.engine_mut().take_authoritative_submit().is_none());
    let agent = tacsim::port::coop::programme_port(rig.world.world())
        .and_then(|p| p.map.vehicles.as_ref())
        .and_then(|v| v.loan_roots.as_ref())
        .ok_or("lender roots")?
        .agent;
    for cap in [
        "operate_crew",
        "deploy",
        "command_assigned_ai",
        "recover_load",
    ] {
        let line = text(
            &mut rig.world,
            agent,
            &format!("@unitgrant/check #{}={cap} delegate", rig.unit.0),
        );
        assert!(line.contains("allowed"), "lessor authority: {line}");
    }
    let request = submit(&mut rig.world, rig.organizer, "+tv/1/permissions go")?;
    assert_ne!(
        request.actor, rig.organizer,
        "the real lessor NPC issues the loans"
    );
    assert_eq!(
        rig.world.world().get(request.actor).map(|o| o.kind),
        Some(ObjKind::Npc)
    );
    stage(&mut rig.world, &store, &request).await?;
    assert!(
        rig.world
            .engine()
            .action_state()
            .get(&h.registry, &())?
            .is_none()
    );
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("agreement")?
        .into_parts();
    assert_eq!(
        bundle.writes().len(),
        1,
        "one authoritative Org registry write, with no personnel ledger"
    );
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    rig.world
        .engine_mut()
        .install_composed_action(installation)?;
    let registry = rig
        .world
        .engine()
        .action_state()
        .get(&h.registry, &())?
        .ok_or("committed loans")?;
    assert_eq!(registry.grants.len(), 4);
    for grant in registry.grants.values() {
        assert_eq!(grant.grantor, request.actor);
        assert_eq!(grant.grantee, UnitGrantee::Character(rig.organizer));
        assert_eq!(
            grant.delegable,
            matches!(
                grant.capability,
                UnitCapability::OperateCrew | UnitCapability::CommandAssignedAi
            )
        );
        assert!(
            matches!(&grant.parent,UnitGrantParent::LeasedOrganizationGrant{lease,..} if lease.unit==rig.unit && lease.tenant==rig.organizer)
        );
    }
    let report = text(&mut rig.world, rig.organizer, "+tv/1/permissions");
    assert_eq!(report.matches("allowed now").count(), 4, "{report}");
    ordinary(&mut rig.world, rig.organizer, "+tv/1/permissions go")?;
    assert!(
        rig.world.engine_mut().take_authoritative_submit().is_none(),
        "retries never duplicate the agreement"
    );
    persist_legacy(&mut rig.world, &store).await?;
    let captain = rig.organizer;
    let now = rig.world.world().now();
    drop(rig);
    let mut loaded = fixture_keyed(&store, true).await?;
    loaded.world.set_now(now);
    let report = text(&mut loaded.world, captain, "+tv/1/permissions");
    assert_eq!(report.matches("allowed now").count(), 4, "{report}");
    commit(&mut loaded.world, &store, agent, "@unitgrant/revoke 4").await?;
    let report = text(&mut loaded.world, captain, "+tv/1/permissions go");
    assert_eq!(report.matches("allowed now").count(), 3, "{report}");
    assert!(
        loaded
            .world
            .engine_mut()
            .take_authoritative_submit()
            .is_none(),
        "retry must not revive a revoked loan"
    );
    assert_eq!(
        loaded
            .world
            .engine()
            .action_state()
            .get(
                &loaded
                    .world
                    .world()
                    .module_state::<UnitAuthorityHandles>()
                    .ok_or("reloaded grants")?
                    .registry,
                &()
            )?
            .ok_or("loans")?
            .grants
            .len(),
        4
    );
    Ok(())
}
#[tokio::test]
async fn returning_the_actual_rental_before_final_take_refuses_the_entire_permission_agreement()
-> TestResult {
    use mush_org::unit_authority::runtime::UnitAuthorityHandles;
    let store = InMemoryStore::new();
    let (mut rig, _) = rented_mining_fixture(&store).await?;
    let request = submit(&mut rig.world, rig.organizer, "+tv/1/permissions go")?;
    stage(&mut rig.world, &store, &request).await?;
    ordinary(&mut rig.world, rig.organizer, "+tv/1 lease return go")?;
    assert!(
        rig.world.engine_mut().take_staged_action().is_err(),
        "the ended real lease invalidates the final owner check"
    );
    let h = rig
        .world
        .world()
        .module_state::<UnitAuthorityHandles>()
        .ok_or("unit grants")?;
    assert!(
        rig.world
            .engine()
            .action_state()
            .get(&h.registry, &())?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn changing_the_last_lending_root_refuses_the_whole_rental_permission_agreement() -> TestResult
{
    use mush_org::unit_authority::{UnitCapability, runtime::UnitAuthorityHandles};
    let store = InMemoryStore::new();
    let (mut rig, _) = rented_mining_fixture(&store).await?;
    let root = *tacsim::port::coop::programme_port(rig.world.world())
        .and_then(|p| p.map.vehicles.as_ref())
        .and_then(|v| v.loan_roots.as_ref())
        .and_then(|r| r.grants.get(&UnitCapability::RecoverLoad))
        .ok_or("last lender root")?;
    let request = submit(&mut rig.world, rig.organizer, "+tv/1/permissions go")?;
    stage(&mut rig.world, &store, &request).await?;
    // Existing Org owner mutation, between preparation and final take. The
    // first proposed loan still has its root; the fourth one no longer does.
    let mut out = mush_core::Outbox::new();
    {
        let e = rig.world.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut e.world,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &e.registry,
            module: Some("org".into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        mush_org::submit(
            &mut ctx,
            &mush_org::OrgMutationPlan::of(vec![mush_org::OrgMutationOp::RevokeGrant {
                grant: root,
            }]),
            mush_org::OrgAction {
                action: "test.lender_revoke".into(),
                org: None,
                detail: json!({"grant":root}),
            },
        )?;
    }
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    let h = rig
        .world
        .world()
        .module_state::<UnitAuthorityHandles>()
        .ok_or("grants")?;
    assert!(
        rig.world
            .engine()
            .action_state()
            .get(&h.registry, &())?
            .is_none()
    );
    Ok(())
}

#[tokio::test]
async fn a_rental_permission_proposal_cannot_be_committed_as_the_lessee() -> TestResult {
    let store = InMemoryStore::new();
    let (mut rig, _) = rented_mining_fixture(&store).await?;
    let mut request = submit(&mut rig.world, rig.organizer, "+tv/1/permissions go")?;
    request.actor = rig.organizer;
    assert!(
        stage(&mut rig.world, &store, &request).await.is_err(),
        "the advisory named lender never replaces the actual source actor"
    );
    Ok(())
}
