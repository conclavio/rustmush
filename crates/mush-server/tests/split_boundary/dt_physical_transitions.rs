//! Real Sim planner, Core object images, coordinator and durable restart.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mush_core::{
    ArgShape, BehaviorBinding, BehaviorId, CmdError, CommandCompatibility, CommandSpec, Ctx,
    Engine, EventHook, GOD, GameModule, HookPoint, HookReadContext, ModError, ObjId, ObjKind,
    OfferKeyRing, OfferSecret, Registrar, ReplayWrite,
};
use mush_proto::RichText;
use mush_server::action_coordinator::{ActionCoordinator, ActionPreflight};
use mush_sim::{SpaceAnchor, SpatialParent, SpatialState, Transition};
use mush_store::{AuthoritativeStore, InMemoryStore};
use mush_testkit::{TestWorld, text_of};

use super::dt_unit_authority::{persist_legacy, stage, submit};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

// A thin test ingress adapter calls the production typed helper. It supplies no
// authority, object images, participant patches or replacement planner.
struct MovementIngress(Arc<AtomicBool>, Arc<AtomicBool>);
struct ForeignVeto(Arc<AtomicBool>, Arc<AtomicBool>);
impl EventHook for ForeignVeto {
    fn carry_attempt(
        &self,
        _: &HookReadContext<'_>,
        _: ObjId,
        _: ObjId,
        _: ObjId,
    ) -> Result<(), RichText> {
        if self.0.load(Ordering::SeqCst) {
            Err(RichText::plain("foreign owner veto"))
        } else {
            Ok(())
        }
    }
    fn move_attempt(
        &self,
        _: &HookReadContext<'_>,
        _: ObjId,
        _: Option<ObjId>,
        _: ObjId,
    ) -> Result<(), RichText> {
        if self.1.load(Ordering::SeqCst) {
            Err(RichText::plain("foreign owner veto"))
        } else {
            Ok(())
        }
    }
}
fn object(raw: &str) -> Result<ObjId, CmdError> {
    raw.trim()
        .trim_start_matches('#')
        .parse::<u32>()
        .map(ObjId)
        .map_err(|_| CmdError::msg("object id required"))
}
impl GameModule for MovementIngress {
    fn name(&self) -> &'static str {
        "dt_physical_test"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(CommandCompatibility::Authoritative);
        reg.command(
            CommandSpec {
                name: "@dt-move".into(),
                shape: ArgShape::EqPair,
                switches: vec!["spoof".into()],
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                let obj = object(ctx.args.lhs())?;
                let space = object(ctx.args.rhs().unwrap_or_default())?;
                if ctx.switches.has("spoof") {
                    // A composed caller cannot replace Core's dispatch actor by
                    // changing the compatibility field before invoking Sim.
                    ctx.actor = GOD;
                }
                mush_sim::physical::submit(
                    ctx,
                    Transition::Place {
                        obj,
                        parent: SpatialParent::Interior { space, slot: None },
                    },
                )
                .map_err(CmdError::msg)
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "@dt-reanchor".into(),
                shape: ArgShape::EqPair,
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                let space = object(ctx.args.lhs())?;
                let host = object(ctx.args.rhs().unwrap_or_default())?;
                mush_sim::physical::submit(
                    ctx,
                    Transition::Space {
                        space,
                        anchor: SpaceAnchor::MovingObject { host },
                    },
                )
                .map_err(CmdError::msg)
            }),
        )?;
        let hook = Arc::new(ForeignVeto(self.0.clone(), self.1.clone()));
        reg.hook(HookPoint::CarryAttempt, hook.clone());
        reg.hook(HookPoint::MoveAttempt, hook);
        Ok(())
    }
}

struct Rig {
    world: TestWorld,
    left: ObjId,
    right: ObjId,
    unit: ObjId,
    carrier: ObjId,
    outsider: ObjId,
    veto: Arc<AtomicBool>,
    actor_veto: Arc<AtomicBool>,
}
fn state(world: &TestWorld) -> TestResult<&SpatialState> {
    world
        .world()
        .module_state::<SpatialState>()
        .ok_or_else(|| "spatial state".into())
}
fn ordinary(world: &mut TestWorld, command: &str) -> TestResult {
    let events = world.run(GOD, command);
    if events.iter().any(|(_, event)| {
        matches!(event, mush_proto::OutputEvent::Sys(text) if text.spans().iter().any(|span| span.tag.as_deref() == Some("error")))
    }) {
        return Err(format!("{command}: {:?}", events.iter().map(|(_, e)| text_of(e)).collect::<Vec<_>>()).into());
    }
    Ok(())
}
async fn fixture(store: &InMemoryStore) -> TestResult<Rig> {
    let mut config = mush_config::config::Config::default();
    config.tacsim.enabled = true;
    let llm = Arc::new(mush_llm::LlmService::from_config(
        &mush_llm::LlmConfig::default(),
    )?);
    let mut modules = mush_server::boot::game_modules(&config, &llm)?;
    let veto = Arc::new(AtomicBool::new(false));
    let actor_veto = Arc::new(AtomicBool::new(false));
    modules.push(Box::new(MovementIngress(veto.clone(), actor_veto.clone())));
    let mut world = mush_server::persist::load_or_seed_world(store).await?;
    let mut ensure = |name: &str, kind| -> TestResult<ObjId> {
        let existing = world.iter().find(|o| o.name == name).map(|o| o.id);
        Ok(match existing {
            Some(id) => id,
            None => world.create(kind, name, GOD, Some(mush_core::LIMBO))?,
        })
    };
    let field = ensure("Prepared movement field", ObjKind::Room)?;
    let left = ensure("Prepared movement left bay", ObjKind::Room)?;
    let right = ensure("Prepared movement right bay", ObjKind::Room)?;
    let unit = ensure("Prepared movement unit", ObjKind::Thing)?;
    let carrier = ensure("Prepared movement carrier", ObjKind::Thing)?;
    let outsider = ensure("Prepared movement outsider", ObjKind::Player)?;
    let mut engine =
        Engine::with_modules(world, mush_server::world_task::base_registry(), modules)?;
    mush_server::modules::boot_modules(&mut engine, store).await?;
    engine.world.set_now(100);
    engine.set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x53; 32])?, [])?);
    let mut world = TestWorld::from_engine(engine);
    if !state(&world)?.spaces.contains_key(&left) {
        for (object, behavior) in [
            (field, "sim.domain"),
            (left, "sim.interior"),
            (right, "sim.interior"),
            (unit, "sim.participant"),
            (carrier, "sim.participant"),
        ] {
            world.world_mut().attach_behavior(
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
            &mut world,
            &format!("@domain/create #{}=sim.passive", field.0),
        )?;
        let domain = state(&world)?
            .domains
            .values()
            .find(|d| d.presentation_location == field)
            .ok_or("domain")?
            .id;
        for command in [
            format!("@spatial/space #{}=fixed:{}", left.0, domain.0),
            format!("@spatial/space #{}=fixed:{}", right.0, domain.0),
            format!("@spatial/place/domain #{}={}", carrier.0, domain.0),
            format!("@spatial/place/interior #{}=#{}", unit.0, left.0),
        ] {
            ordinary(&mut world, &command)?;
        }
    }
    persist_legacy(&mut world, store).await?;
    Ok(Rig {
        world,
        left,
        right,
        unit,
        carrier,
        outsider,
        veto,
        actor_veto,
    })
}
fn assert_location(rig: &Rig, room: ObjId) -> TestResult {
    assert_eq!(
        rig.world.world().get(rig.unit).ok_or("unit")?.location,
        Some(room)
    );
    assert_eq!(
        state(&rig.world)?
            .placements
            .get(&rig.unit)
            .ok_or("placement")?
            .parent,
        SpatialParent::Interior {
            space: room,
            slot: None
        }
    );
    assert_eq!(
        rig.world
            .world()
            .get(room)
            .ok_or("room")?
            .contents
            .iter()
            .filter(|id| **id == rig.unit)
            .count(),
        1
    );
    let other = if room == rig.left {
        rig.right
    } else {
        rig.left
    };
    assert!(
        !rig.world
            .world()
            .get(other)
            .ok_or("other room")?
            .contents
            .contains(&rig.unit)
    );
    Ok(())
}

#[tokio::test]
async fn lost_ack_recovers_object_and_spatial_images_together_and_retry_returns_receipt()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let line = format!("@dt-move {}={}", rig.unit.0, rig.right.0);
    let submission = submit(&mut rig.world, GOD, &line)?;
    let ingress = stage(&mut rig.world, &store, &submission).await?;
    assert_location(&rig, rig.left)?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("movement")?
        .into_parts();
    assert_eq!(
        bundle
            .writes()
            .iter()
            .filter(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
            .count(),
        3
    );
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    assert_location(&rig, rig.left)?;
    assert!(matches!(
        ActionCoordinator::preflight(&store, ingress.clone()).await?,
        ActionPreflight::Pending { .. }
    ));
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert_location(&rig, rig.left)?;
    drop(installation);
    drop(rig);
    let restarted = fixture(&store).await?;
    assert_location(&restarted, restarted.right)?;
    let ActionPreflight::Finalized(retry) = ActionCoordinator::preflight(&store, ingress).await?
    else {
        return Err("exact retry did not return receipt".into());
    };
    assert_eq!(retry.token(), bundle.token());
    assert_eq!(retry.outcome(), receipt.outcome());
    Ok(())
}

#[tokio::test]
async fn final_take_refuses_changed_object_preimages_and_foreign_hook_vetoes() -> TestResult {
    for veto in [false, true] {
        let store = InMemoryStore::new();
        let mut rig = fixture(&store).await?;
        let line = format!("@dt-move {}={}", rig.unit.0, rig.right.0);
        let submission = submit(&mut rig.world, GOD, &line)?;
        stage(&mut rig.world, &store, &submission).await?;
        if veto {
            rig.veto.store(true, Ordering::SeqCst);
        } else {
            rig.world.world_mut().get_mut(rig.unit).ok_or("unit")?.name =
                "Changed after preparation".into();
        }
        assert!(rig.world.engine_mut().take_staged_action().is_err());
        assert_location(&rig, rig.left)?;
    }
    Ok(())
}

#[tokio::test]
async fn zero_projection_reanchor_is_durable_and_non_admin_cannot_use_the_helper() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let move_line = format!("@dt-move {}={}", rig.unit.0, rig.right.0);
    let _ = rig.world.run(rig.outsider, &move_line);
    assert!(rig.world.engine_mut().take_authoritative_submit().is_none());
    let spoof_line = format!("@dt-move/spoof {}={}", rig.unit.0, rig.right.0);
    let spoof_events = rig.world.run(rig.outsider, &spoof_line);
    assert!(spoof_events.iter().any(|(_, event)| {
        text_of(event).contains("Structural request requires the actual command actor.")
    }));
    assert!(rig.world.engine_mut().take_authoritative_submit().is_none());
    assert_location(&rig, rig.left)?;
    let before_objects = serde_json::to_value(rig.world.world().iter().collect::<Vec<_>>())?;
    let line = format!("@dt-reanchor {}={}", rig.left.0, rig.carrier.0);
    let submission = submit(&mut rig.world, GOD, &line)?;
    stage(&mut rig.world, &store, &submission).await?;
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("reanchor")?
        .into_parts();
    assert!(
        !bundle
            .writes()
            .iter()
            .any(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
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
    assert_eq!(
        before_objects,
        serde_json::to_value(rig.world.world().iter().collect::<Vec<_>>())?
    );
    assert_eq!(
        state(&rig.world)?
            .spaces
            .get(&rig.left)
            .ok_or("space")?
            .anchor,
        SpaceAnchor::MovingObject { host: rig.carrier }
    );
    drop(rig);
    let restarted = fixture(&store).await?;
    assert_location(&restarted, restarted.left)?;
    assert_eq!(
        state(&restarted.world)?
            .spaces
            .get(&restarted.left)
            .ok_or("space")?
            .anchor,
        SpaceAnchor::MovingObject {
            host: restarted.carrier
        }
    );
    Ok(())
}

#[tokio::test]
async fn actor_projection_rechecks_the_foreign_movement_veto_independently_of_carry() -> TestResult
{
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let npc =
        rig.world
            .world_mut()
            .create(ObjKind::Npc, "Moving existing crew", GOD, Some(rig.left))?;
    rig.world.world_mut().attach_behavior(
        npc,
        BehaviorId::new("sim.participant")?,
        BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 0,
        },
    )?;
    ordinary(
        &mut rig.world,
        &format!("@spatial/place/interior #{}=#{}", npc.0, rig.left.0),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    let submission = submit(
        &mut rig.world,
        GOD,
        &format!("@dt-move {}={}", npc.0, rig.right.0),
    )?;
    stage(&mut rig.world, &store, &submission).await?;
    rig.actor_veto.store(true, Ordering::SeqCst);
    assert!(rig.world.engine_mut().take_staged_action().is_err());
    assert_eq!(
        rig.world.world().get(npc).ok_or("crew")?.location,
        Some(rig.left)
    );
    assert_eq!(
        state(&rig.world)?
            .placements
            .get(&npc)
            .ok_or("crew placement")?
            .parent,
        SpatialParent::Interior {
            space: rig.left,
            slot: None
        }
    );
    Ok(())
}

#[tokio::test]
async fn late_sim_drift_refuses_installation_without_publishing_the_prepared_receipt() -> TestResult
{
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let line = format!("@dt-reanchor {}={}", rig.left.0, rig.carrier.0);
    let submission = submit(&mut rig.world, GOD, &line)?;
    stage(&mut rig.world, &store, &submission).await?;
    let before_revision = rig.world.engine().action_state().revision();
    let before_records = rig.world.engine().action_state().len();
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("reanchor")?
        .into_parts();
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    // Deliberately bypass the server's writer-custody command fence to emulate
    // late owner drift. The real legacy Sim command changes no object location.
    ordinary(
        &mut rig.world,
        &format!("@spatial/space #{}=unanchored", rig.left.0),
    )?;
    assert_eq!(
        state(&rig.world)?
            .spaces
            .get(&rig.left)
            .ok_or("space")?
            .anchor,
        SpaceAnchor::Unanchored
    );
    assert!(
        rig.world
            .engine_mut()
            .install_composed_action(installation)
            .is_err()
    );
    assert_eq!(
        state(&rig.world)?
            .spaces
            .get(&rig.left)
            .ok_or("space")?
            .anchor,
        SpaceAnchor::Unanchored
    );
    assert_eq!(
        rig.world.engine().action_state().revision(),
        before_revision
    );
    assert_eq!(rig.world.engine().action_state().len(), before_records);
    drop(rig);
    let restarted = fixture(&store).await?;
    assert_eq!(
        state(&restarted.world)?
            .spaces
            .get(&restarted.left)
            .ok_or("space")?
            .anchor,
        SpaceAnchor::MovingObject {
            host: restarted.carrier
        }
    );
    Ok(())
}

#[tokio::test]
async fn zero_projection_retains_proposed_host_world_preimage_through_installation() -> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let original_anchor = state(&rig.world)?
        .spaces
        .get(&rig.left)
        .ok_or("space")?
        .anchor
        .clone();
    let line = format!("@dt-reanchor {}={}", rig.left.0, rig.carrier.0);
    let submission = submit(&mut rig.world, GOD, &line)?;
    stage(&mut rig.world, &store, &submission).await?;
    let before_revision = rig.world.engine().action_state().revision();
    let before_records = rig.world.engine().action_state().len();
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("reanchor")?
        .into_parts();
    assert!(
        !bundle
            .writes()
            .iter()
            .any(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
    );
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    // Emulate a writer-custody violation affecting the proposed carrier, whose
    // World image is read-only in this zero-projection structural change.
    rig.world
        .world_mut()
        .get_mut(rig.carrier)
        .ok_or("carrier")?
        .flags
        .insert(mush_core::Flags::GOING);
    assert!(
        rig.world
            .engine_mut()
            .install_composed_action(installation)
            .is_err()
    );
    assert_eq!(
        state(&rig.world)?
            .spaces
            .get(&rig.left)
            .ok_or("space")?
            .anchor,
        original_anchor
    );
    assert_eq!(
        rig.world.engine().action_state().revision(),
        before_revision
    );
    assert_eq!(rig.world.engine().action_state().len(), before_records);
    Ok(())
}
