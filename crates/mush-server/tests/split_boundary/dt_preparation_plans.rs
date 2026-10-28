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
    let submission = submit(world, actor, line)?;
    stage(world, store, &submission).await?;
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

fn seated_fixture(rig: &mut Rig) -> TestResult<ObjId> {
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
    for command in [
        format!("@spatial/place/domain #{}={}", rig.carrier.0, domain.0),
        format!("@spatial/space #{}=moving:#{}", bay.0, rig.carrier.0),
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
    let bay = seated_fixture(&mut rig)?;
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
