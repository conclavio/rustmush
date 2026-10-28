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
use mush_store::{AuthoritativeStore, InMemoryStore, ObjectStore};
use mush_testkit::{TestWorld, text_of};
use serde_json::{Value, json};

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
        ] {
            commit_physical(&mut world, store, &command).await?;
        }
        for command in [
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
    let line = format!("@spatial/space #{}=moving:#{}", rig.left.0, rig.carrier.0);
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
    let line = format!("@spatial/space #{}=moving:#{}", rig.left.0, rig.carrier.0);
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
    // Deliberately bypass the server's writer-custody fence with an existing
    // legacy owner command. Compartment authoring changes the canonical space
    // record without changing any object location.
    ordinary(
        &mut rig.world,
        &format!("@spatial/slot #{}=late-drift", rig.left.0),
    )?;
    let slot = mush_sim::SlotId::new("late-drift").ok_or("slot")?;
    assert!(
        state(&rig.world)?
            .spaces
            .get(&rig.left)
            .ok_or("space")?
            .compartments
            .contains_key(&slot)
    );
    assert!(
        rig.world
            .engine_mut()
            .install_composed_action(installation)
            .is_err()
    );
    assert!(
        state(&rig.world)?
            .spaces
            .get(&rig.left)
            .ok_or("space")?
            .compartments
            .contains_key(&slot)
    );
    assert_eq!(
        rig.world.engine().action_state().revision(),
        before_revision
    );
    assert_eq!(rig.world.engine().action_state().len(), before_records);
    drop(rig);
    let restarted = fixture(&store).await?;
    assert!(
        !state(&restarted.world)?
            .spaces
            .get(&restarted.left)
            .ok_or("space")?
            .compartments
            .contains_key(&slot)
    );
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
    let line = format!("@spatial/space #{}=moving:#{}", rig.left.0, rig.carrier.0);
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

fn capture_write(bundle: &mush_core::AuthoritativeBundle) -> TestResult<(String, Value)> {
    let rows: Vec<_> = bundle
        .writes()
        .iter()
        .filter_map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "sim"
                && key
                    .as_str()
                    .starts_with("action-state/v1/personnel_crossings/") =>
            {
                Some((key.to_string(), value.clone()))
            }
            _ => None,
        })
        .collect();
    if rows.len() != 1 {
        return Err("physical v2 commit requires exactly one capture".into());
    }
    rows.into_iter().next().ok_or_else(|| "capture".into())
}

async fn commit_physical(
    world: &mut TestWorld,
    store: &InMemoryStore,
    line: &str,
) -> TestResult<Value> {
    persist_legacy(world, store).await?;
    let submission = submit(world, GOD, line)?;
    stage(world, store, &submission).await?;
    let (bundle, installation) = world
        .engine_mut()
        .take_staged_action()?
        .ok_or("movement")?
        .into_parts();
    let (_, capture) = capture_write(&bundle)?;
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    world.engine_mut().install_composed_action(installation)?;
    Ok(capture)
}

async fn commit_movement(rig: &mut Rig, store: &InMemoryStore, line: &str) -> TestResult<Value> {
    commit_physical(&mut rig.world, store, line).await
}

#[tokio::test]
async fn space_command_creates_only_after_ack_and_projects_the_same_receipt_after_restart()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let cabin = rig
        .world
        .world_mut()
        .create(ObjKind::Room, "New crew cabin", GOD, None)?;
    let crew = rig
        .world
        .world_mut()
        .create(ObjKind::Npc, "Actual cabin crew", GOD, Some(cabin))?;
    rig.world.world_mut().attach_behavior(
        cabin,
        BehaviorId::new("sim.interior")?,
        BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 0,
        },
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    let line = format!("@spatial/space #{}=moving:#{}", cabin.0, rig.carrier.0);
    let refused = rig.world.run(rig.outsider, &line);
    assert!(!refused.is_empty());
    assert!(!rig.world.engine().has_authoritative_submit());
    assert!(!state(&rig.world)?.spaces.contains_key(&cabin));

    let events = rig.world.run(GOD, &line);
    assert!(
        !events
            .iter()
            .any(|(_, event)| text_of(event).contains("committed"))
    );
    assert!(!state(&rig.world)?.spaces.contains_key(&cabin));
    // A second command cannot replace the pending transition.
    let duplicate = rig
        .world
        .run(GOD, &format!("@spatial/space #{}=unanchored", cabin.0));
    assert!(
        duplicate
            .iter()
            .any(|(_, event)| text_of(event).contains("pending"))
    );
    let submission = rig
        .world
        .engine_mut()
        .take_authoritative_submit()
        .ok_or("space request")?;
    assert_eq!(
        submission.action.key().id.as_str(),
        "sim_physical_transition"
    );
    let ingress = stage(&mut rig.world, &store, &submission).await?;
    assert!(!state(&rig.world)?.spaces.contains_key(&cabin));
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("space commit")?
        .into_parts();
    let (capture_key, capture) = capture_write(&bundle)?;
    assert_eq!(
        capture["value"]["crossings"],
        json!([
            {"character":crew,"carrier":rig.carrier,"direction":"enter"}
        ])
    );
    assert!(
        !bundle
            .writes()
            .iter()
            .any(|write| matches!(write, ReplayWrite::ObjectSet { .. }))
    );
    assert!(store.module_json_get("sim", &capture_key).await?.is_none());
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    let principal = mush_core::PrincipalId::Object { id: GOD };
    let status = ActionCoordinator::status_id(ingress.idempotency())?;
    let projections = rig
        .world
        .engine()
        .registry
        .action_providers()
        .ok_or("providers")?
        .receipt_projection_table();
    assert!(matches!(
        ActionCoordinator::projected_status(&store, &projections, &principal, &status).await?,
        mush_proto::ProjectedActionStatus::Pending { .. }
    ));
    let receipt = store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert!(!state(&rig.world)?.spaces.contains_key(&cabin));
    let projected = ActionCoordinator::project_finalized_receipt(
        &projections,
        &principal,
        status.clone(),
        &receipt,
    )?;
    assert_eq!(
        projected.result().ok_or("space summary")?.summary(),
        format!("Spatial update for {cabin} committed.")
    );
    assert_eq!(
        store.module_json_get("sim", &capture_key).await?,
        Some(capture)
    );

    // Lose the acknowledgement. Fresh production boot restores only the
    // committed owner records and projects the identical immutable receipt.
    drop(installation);
    drop(rig);
    let restarted = fixture(&store).await?;
    assert_eq!(
        state(&restarted.world)?
            .spaces
            .get(&cabin)
            .ok_or("committed cabin")?
            .anchor,
        SpaceAnchor::MovingObject {
            host: restarted.carrier
        }
    );
    assert_eq!(
        restarted.world.world().get(crew).ok_or("crew")?.location,
        Some(cabin)
    );
    let ActionPreflight::Finalized(retry) = ActionCoordinator::preflight(&store, ingress).await?
    else {
        return Err("space retry must return committed receipt".into());
    };
    let projections = restarted
        .world
        .engine()
        .registry
        .action_providers()
        .ok_or("providers")?
        .receipt_projection_table();
    assert_eq!(
        ActionCoordinator::project_finalized_receipt(
            &projections,
            &principal,
            status.clone(),
            &retry
        )?,
        projected
    );
    assert!(
        ActionCoordinator::project_finalized_receipt(
            &projections,
            &mush_core::PrincipalId::Object {
                id: restarted.outsider
            },
            status,
            &retry,
        )
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn personnel_capture_zero_projection_lost_ack_retains_exact_preparation_clock_and_retry()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let walker = rig.world.world_mut().create(
        ObjKind::Npc,
        "Existing unplaced crew",
        GOD,
        Some(rig.left),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    let line = format!("@spatial/space #{}=moving:#{}", rig.left.0, rig.carrier.0);
    let submission = submit(&mut rig.world, GOD, &line)?;
    rig.world.world_mut().set_now(101);
    let prepared_at = rig.world.world().world_ic_now();
    let ingress = stage(&mut rig.world, &store, &submission).await?;
    let before_records = rig.world.engine().action_state().len();
    rig.world.world_mut().set_now(102);
    let (bundle, installation) = rig
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("movement")?
        .into_parts();
    let (key, capture) = capture_write(&bundle)?;
    assert_eq!(
        capture["value"]["prepared_at"],
        serde_json::to_value(&prepared_at)?
    );
    assert_eq!(
        capture["value"]["coverage"],
        json!("uncertified_legacy_paths")
    );
    assert_eq!(
        capture["value"]["crossings"],
        json!([
            {"character":walker,"carrier":rig.carrier,"direction":"enter"}
        ])
    );
    assert!(
        !bundle
            .writes()
            .iter()
            .any(|w| matches!(w, ReplayWrite::ObjectSet { .. }))
    );
    assert!(store.module_json_get("sim", &key).await?.is_none());
    store
        .reserve_authoritative_intent((*bundle).clone())
        .await?;
    assert_eq!(rig.world.engine().action_state().len(), before_records);
    store
        .reconcile_authoritative(bundle.token(), bundle.bundle_hash())
        .await?;
    assert_eq!(rig.world.engine().action_state().len(), before_records);
    drop(installation);
    drop(rig);
    let restarted = fixture(&store).await?;
    assert_eq!(
        restarted.world.world().get(walker).ok_or("crew")?.location,
        Some(restarted.left)
    );
    assert_eq!(store.module_json_get("sim", &key).await?, Some(capture));
    let ActionPreflight::Finalized(receipt) = ActionCoordinator::preflight(&store, ingress).await?
    else {
        return Err("exact retry did not finalize".into());
    };
    assert_eq!(receipt.token(), bundle.token());
    Ok(())
}

#[tokio::test]
async fn personnel_capture_same_carrier_walk_is_empty_and_reentry_is_a_distinct_uncertified_commit()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let npc = rig.world.world_mut().create(
        ObjKind::Npc,
        "Existing assigned crew",
        GOD,
        Some(rig.left),
    )?;
    rig.world.world_mut().attach_behavior(
        npc,
        BehaviorId::new("sim.participant")?,
        BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 0,
        },
    )?;
    for command in [
        format!("@spatial/space #{}=moving:#{}", rig.left.0, rig.carrier.0),
        format!("@spatial/space #{}=moving:#{}", rig.right.0, rig.carrier.0),
    ] {
        commit_physical(&mut rig.world, &store, &command).await?;
    }
    ordinary(
        &mut rig.world,
        &format!("@spatial/place/interior #{}=#{}", npc.0, rig.left.0),
    )?;
    persist_legacy(&mut rig.world, &store).await?;
    let target = rig.right;
    let walk = commit_movement(
        &mut rig,
        &store,
        &format!("@dt-move {}={}", npc.0, target.0),
    )
    .await?;
    assert_eq!(walk["value"]["crossings"], json!([]));
    // Raw legacy movement still bypasses capture. Leaving and returning cannot
    // certify continuity or upgrade this slice's explicitly uncertified data.
    let prior_records = rig.world.engine().action_state().len();
    rig.world.world_mut().move_to(npc, mush_core::LIMBO)?;
    rig.world.world_mut().move_to(npc, rig.right)?;
    assert_eq!(rig.world.engine().action_state().len(), prior_records);
    persist_legacy(&mut rig.world, &store).await?;
    let second_carrier = rig.world.world_mut().create(
        ObjKind::Thing,
        "Second carrier",
        GOD,
        Some(mush_core::LIMBO),
    )?;
    rig.world.world_mut().attach_behavior(
        second_carrier,
        BehaviorId::new("sim.participant")?,
        BehaviorBinding {
            schema_version: 1,
            attached_by: GOD,
            attached_at: 0,
        },
    )?;
    let domain = state(&rig.world)?
        .domains
        .values()
        .next()
        .ok_or("domain")?
        .id;
    ordinary(
        &mut rig.world,
        &format!("@spatial/place/domain #{}={}", second_carrier.0, domain.0),
    )?;
    commit_physical(
        &mut rig.world,
        &store,
        &format!(
            "@spatial/space #{}=moving:#{}",
            rig.left.0, second_carrier.0
        ),
    )
    .await?;
    persist_legacy(&mut rig.world, &store).await?;
    let target = rig.left;
    let transfer = commit_movement(
        &mut rig,
        &store,
        &format!("@dt-move {}={}", npc.0, target.0),
    )
    .await?;
    assert_eq!(
        transfer["value"]["crossings"],
        json!([
            {"character":npc,"carrier":rig.carrier,"direction":"exit"},
            {"character":npc,"carrier":second_carrier,"direction":"enter"}
        ])
    );
    let target = rig.right;
    let returned = commit_movement(
        &mut rig,
        &store,
        &format!("@dt-move {}={}", npc.0, target.0),
    )
    .await?;
    assert_eq!(
        returned["value"]["crossings"],
        json!([
            {"character":npc,"carrier":rig.carrier,"direction":"enter"},
            {"character":npc,"carrier":second_carrier,"direction":"exit"}
        ])
    );
    assert_ne!(transfer["value"]["commit"], returned["value"]["commit"]);
    for capture in [&walk, &transfer, &returned] {
        assert_eq!(
            capture["value"]["coverage"],
            json!("uncertified_legacy_paths")
        );
        assert!(capture["value"].get("eligible").is_none());
        assert!(capture["value"].get("interval").is_none());
    }
    Ok(())
}

#[tokio::test]
async fn personnel_capture_final_take_refuses_clock_generation_change_or_backward_time()
-> TestResult {
    for generation_change in [false, true] {
        let store = InMemoryStore::new();
        let mut rig = fixture(&store).await?;
        let line = format!("@spatial/space #{}=moving:#{}", rig.left.0, rig.carrier.0);
        let submission = submit(&mut rig.world, GOD, &line)?;
        rig.world.world_mut().set_now(101);
        stage(&mut rig.world, &store, &submission).await?;
        let before = rig.world.engine().action_state().len();
        if generation_change {
            let epoch = rig.world.world().next_world_timeline_epoch(2)?;
            rig.world.world_mut().append_world_timeline_epoch(epoch)?;
        } else {
            rig.world.world_mut().set_now(100);
        }
        assert!(rig.world.engine_mut().take_staged_action().is_err());
        assert_eq!(rig.world.engine().action_state().len(), before);
        assert_location(&rig, rig.left)?;
    }
    Ok(())
}

fn audit_captures(records: Vec<mush_core::ActionStateBootRecord>) -> TestResult {
    // Exercise Sim's actual boot audit with precisely the retained typed rows.
    // No replacement policy/provider or direct family mutation is installed.
    let mut engine = Engine::with_modules(
        mush_core::World::new_minimal(),
        mush_core::Registry::new(),
        vec![Box::new(mush_sim::SimulationModule::new())],
    )?;
    engine.hydrate_action_state(records)?;
    engine.all_modules_loaded(
        &mut mush_core::Outbox::new(),
        &mut mush_core::Effects::new(),
    )?;
    Ok(())
}

#[tokio::test]
async fn personnel_capture_boot_audit_requires_exact_pair_and_retains_v1_without_backfill()
-> TestResult {
    let store = InMemoryStore::new();
    let mut rig = fixture(&store).await?;
    let line = format!("@spatial/space #{}=moving:#{}", rig.left.0, rig.carrier.0);
    let capture = commit_movement(&mut rig, &store, &line).await?;
    let rows: Vec<_> = store
        .module_json_list("sim")
        .await?
        .into_iter()
        .filter(|(key, value)| {
            (key.starts_with("action-state/v1/personnel_crossings/")
                || key.starts_with("action-state/v1/physical_transitions/"))
                && value["value"]["commit"] == capture["value"]["commit"]
        })
        .map(|(key, value)| mush_core::ActionStateBootRecord::new("sim", key, value))
        .collect();
    assert_eq!(rows.len(), 2);
    audit_captures(rows.clone())?;
    let capture_index = rows
        .iter()
        .position(|row| row.key.contains("/personnel_crossings/"))
        .ok_or("capture")?;
    let physical_index = rows
        .iter()
        .position(|row| row.key.contains("/physical_transitions/"))
        .ok_or("physical")?;
    assert!(audit_captures(vec![rows.get(capture_index).ok_or("capture")?.clone()]).is_err());
    let mut missing = rows.clone();
    missing.remove(capture_index);
    assert!(audit_captures(missing).is_err());
    for field in ["operator", "root"] {
        let mut mismatched = rows.clone();
        mismatched.get_mut(capture_index).ok_or("capture")?.value["value"][field] = json!(999999);
        assert!(audit_captures(mismatched).is_err());
    }
    let mut cross_key = rows.clone();
    let row = cross_key.get_mut(capture_index).ok_or("capture")?;
    let other = mush_core::CommitToken::from_bytes([0x91; 16]).to_string();
    row.key = format!("action-state/v1/personnel_crossings/{other}").into();
    row.value["key"] = json!(other);
    assert!(audit_captures(cross_key).is_err());

    // A real retained v1 descriptor pair remains historical; migration creates
    // no crossing record from the currently occupied interior.
    let definitions = rig
        .world
        .engine()
        .registry
        .definitions()
        .ok_or("definitions")?;
    let reference = |kind: &str, id: &str| -> TestResult<mush_core::DefinitionRef> {
        definitions
            .iter()
            .find(|entry| {
                let r = entry.reference();
                r.key().owner.as_str() == "sim"
                    && r.key().kind.as_str() == kind
                    && r.key().id.as_str() == id
                    && r.version() == 1
            })
            .map(|entry| entry.reference().clone())
            .ok_or_else(|| "historical definition".into())
    };
    let mut historical = rows.get(physical_index).ok_or("physical")?.clone();
    historical.value["value"]["action"] =
        serde_json::to_value(reference("action", "sim_physical_transition")?)?;
    historical.value["value"]["participant"] =
        serde_json::to_value(reference("owner_participant", "physical_transition")?)?;
    audit_captures(vec![historical.clone()])?;
    assert!(
        audit_captures(vec![
            historical,
            rows.get(capture_index).ok_or("capture")?.clone()
        ])
        .is_err()
    );
    Ok(())
}
