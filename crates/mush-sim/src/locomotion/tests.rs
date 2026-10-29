#![cfg(test)]

use super::*;
use crate::data::{
    CapacitySpec, DomainClock, DomainRecord, DomainStatus, HostMounts, MountId, MountSpec,
    PlacementRecord, WindowBox, WindowOpener, WindowRec, WindowSurface,
};
use mush_core::action::{
    ActionInvocationDraft, ActionOfferId, CanonicalHash, CommitToken, IdempotencyKey,
    IdempotencyScope, PreparedAuthoritativeAction, PreparedCausalRef, RequestUid,
    ScopedIdempotencyKey,
};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::composed::PreparedComposedTxn;
use mush_core::{
    CanonicalId, DefinitionKey, Effects, Engine, GOD, ObjId, ObjKind, Outbox, RevisionSet,
};
use std::collections::BTreeMap;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);

struct Rig {
    engine: Box<Engine>,
    action: DefinitionRef,
    actor: ObjId,
    source: ObjId,
    destination: ObjId,
    exit: ObjId,
}

fn token() -> CommitToken {
    CommitToken::from_bytes([92; 16])
}

impl Rig {
    fn new(kind: ObjKind) -> TestResult<Self> {
        let mut test =
            mush_testkit::TestWorld::with_modules(vec![Box::new(crate::SimulationModule::new())]);
        test.set_now(100);
        let source = test
            .world_mut()
            .create(ObjKind::Room, "source", GOD, None)?;
        let destination = test
            .world_mut()
            .create(ObjKind::Room, "destination", GOD, None)?;
        let actor = test.world_mut().create(kind, "walker", GOD, Some(source))?;
        let exit = test
            .world_mut()
            .create(ObjKind::Exit, "north;n", GOD, Some(source))?;
        test.world_mut().link_exit(exit, Some(destination))?;
        test.world_mut().set_home(actor, Some(destination))?;
        // Fixture setup establishes the initial World and first-boot module
        // state. No prepared operation is installed by these staging tests.
        let _ = test.take_module_ops();
        test.world_mut().take_dirty();
        let engine = Box::new(test.into_engine());
        let action = engine
            .registry
            .definitions()
            .ok_or("definitions absent")?
            .current(&DefinitionKey {
                owner: OwnerId::new(NS)?,
                kind: DefinitionKind::new("action")?,
                id: DefinitionId::new(mush_core::locomotion::transition::ACTION)?,
            })
            .cloned()
            .ok_or("locomotion action absent")?;
        Ok(Self {
            engine,
            action,
            actor,
            source,
            destination,
            exit,
        })
    }

    fn invocation(&self) -> TestResult<ActionInvocation> {
        let request = RequestUid::from_bytes([92; 16]);
        let principal = PrincipalId::Object { id: self.actor };
        Ok(ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([9; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new(NS)?,
                    namespace: CanonicalId::new("locomotion_test")?,
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })?)
    }

    fn prepare(&mut self, line: &str) -> TestResult<Prepared> {
        let mut out = Outbox::default();
        let captured = self
            .engine
            .capture_locomotion_command(self.actor, line, &mut out, &mut Effects::default())
            .ok_or("dispatch did not capture the route")?;
        check!(
            out.is_empty(),
            "capture unexpectedly emitted output: {out:?}"
        );
        Ok(self.engine.prepare_captured_locomotion_action(
            captured,
            &self.action,
            &self.invocation()?,
            token(),
        )?)
    }

    fn stage(&mut self, prepared: Prepared) -> TestResult {
        let invocation = self.invocation()?;
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = prepared.1.into_parts();
        definitions.push(self.action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(),
            request: invocation.request(),
            idempotency: invocation.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: invocation.request_hash(),
            plan_hash: CanonicalHash::from_bytes([2; 32]),
            action: self.action.clone(),
            provider: OwnerId::new(NS)?,
            cause: PreparedCausalRef::new(invocation.request(), None, 0)?,
            definitions,
            writes: prepared.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|resolution| resolution.bind(token())),
            semantic_events,
            eligible_witnesses,
            durable_outbox,
        })?;
        let guards = self
            .engine
            .registry
            .server_guard_set_builder(RevisionSet::default())?
            .finish()?;
        self.engine
            .stage_authoritative_action(prepared.0, guards, bundle)?;
        Ok(())
    }

    fn assert_unpublished(&self) -> TestResult {
        check_eq!(
            self.engine
                .world
                .get(self.actor)
                .and_then(|actor| actor.location),
            Some(self.source)
        );
        check_eq!(
            crate::state(&self.engine.world)
                .ok_or("Sim state absent")?
                .recovery
                .next_seq(RECOVERY_LOG),
            0
        );
        check!(!self.engine.has_legacy_persistence_work());
        Ok(())
    }

    fn add_window(&mut self) -> TestResult {
        let state = self
            .engine
            .world
            .module_state_mut::<SpatialState>()
            .ok_or("Sim state absent")?;
        state.domains.insert(
            DomainId(900),
            DomainRecord {
                id: DomainId(900),
                host: GOD,
                driver: crate::passive_definition_ref(),
                status: DomainStatus::Active,
                clock: DomainClock::Passive,
                presentation_location: mush_core::LIMBO,
                revision: 1,
                window: Some(WindowRec {
                    region: "unrelated-region".into(),
                    surfaces: vec![WindowSurface::Ground],
                    bbox: WindowBox {
                        x0: 0,
                        y0: 0,
                        x1: 100,
                        y1: 100,
                    },
                    cell_cm: 100,
                    composited_from: vec![],
                    features_seen: 0,
                    opened_by: WindowOpener::Staff { by: GOD },
                    revision: 1,
                }),
            },
        );
        Ok(())
    }
}

#[test]
fn real_capture_prepares_objects_audit_and_crossings_without_publication() -> TestResult {
    for kind in [ObjKind::Player, ObjKind::Npc] {
        let mut rig = Rig::new(kind)?;
        let prepared = rig.prepare("n")?;
        let writes = prepared.0.authoritative_replay_writes();
        let objects: BTreeSet<_> = writes
            .iter()
            .filter_map(|write| match write {
                ReplayWrite::ObjectSet { object } => Some(object.id),
                _ => None,
            })
            .collect();
        check_eq!(
            objects,
            BTreeSet::from([rig.actor, rig.source, rig.destination])
        );
        check_eq!(writes.iter().filter(|write| matches!(write,
            ReplayWrite::LedgerChunkSet { write } if write.namespace().as_str() == NS && write.log_id() == RECOVERY_LOG)).count(), 1);
        let typed: BTreeSet<_> = writes
            .iter()
            .filter_map(|write| match write {
                ReplayWrite::ModuleJsonSet { key, .. } => Some(key.as_str().to_owned()),
                _ => None,
            })
            .collect();
        check_eq!(typed.len(), 2);
        check!(
            typed
                .iter()
                .any(|key| key.starts_with("action-state/v1/locomotion_transitions/"))
        );
        check!(
            typed
                .iter()
                .any(|key| key.starts_with("action-state/v1/locomotion_crossings/"))
        );
        rig.assert_unpublished()?;
        rig.stage(prepared)?;
        rig.assert_unpublished()?;
        let staged = rig
            .engine
            .take_staged_action()?
            .ok_or("staged action absent")?;
        let (_, installation) = staged.into_parts();
        rig.engine.abort_prepared_action(installation)?;
        rig.assert_unpublished()?;
        check!(!rig.engine.action_state().has_prepared());
    }
    Ok(())
}

#[test]
fn generic_preparation_cannot_replace_actual_dispatch_capture() -> TestResult {
    let mut rig = Rig::new(ObjKind::Player)?;
    let invocation = rig.invocation()?;
    check!(
        rig.engine
            .prepare_registered_composed_authoritative_action(&rig.action, &invocation, token())
            .is_err()
    );
    rig.assert_unpublished()?;
    check!(!rig.engine.action_state().has_prepared());
    Ok(())
}

#[test]
fn retained_journey_preimage_refuses_post_custody_drift_but_allows_allocator_progress() -> TestResult
{
    for change in 0..3 {
        let mut rig = Rig::new(ObjKind::Player)?;
        let hull = rig
            .engine
            .world
            .create(ObjKind::Thing, "carried hull", GOD, Some(rig.actor))?;
        rig.engine.world.take_dirty();
        let prepared = rig.prepare("north")?;
        rig.stage(prepared)?;
        let (_, install) = rig
            .engine
            .take_staged_action()?
            .ok_or("staged")?
            .into_parts();
        let journey = if change == 1 {
            let route = crate::RouteDef::new(
                CanonicalId::new("test")?,
                (rig.source, rig.destination),
                vec![crate::LegDef::new(rig.destination, 30)?],
                CanonicalId::new("manual")?,
                0,
            )?;
            let now = rig.engine.world.world_ic_now();
            Some(crate::CarrierJourney::new(
                crate::JourneyId(1),
                hull,
                route,
                SpatialParent::Interior {
                    space: rig.destination,
                    slot: None,
                },
                now.clone(),
                now,
                0,
                crate::JourneyState::Cruising,
                None,
            )?)
        } else {
            None
        };
        let book = rig
            .engine
            .world
            .module_state_mut::<crate::JourneyBook>()
            .ok_or("journeys")?;
        match change {
            0 => {
                book.held
                    .insert("cjourney:broken".into(), "retained repair fact".into());
            }
            1 => {
                book.journeys.insert(hull, journey.ok_or("journey")?);
            }
            _ => {
                book.next_id += 1;
                book.cursor += 1;
            }
        }
        let installed = rig.engine.install_composed_action(install);
        if change < 2 {
            check!(installed.is_err());
            check!(rig.engine.action_state().reservation_lost());
            rig.assert_unpublished()?;
        } else {
            installed?;
            check_eq!(
                rig.engine.world.get(rig.actor).and_then(|o| o.location),
                Some(rig.destination)
            );
            check!(!rig.engine.action_state().reservation_lost());
        }
    }
    Ok(())
}

#[test]
fn self_loop_exit_keeps_all_object_bytes_and_still_prepares_real_audit() -> TestResult {
    let mut rig = Rig::new(ObjKind::Player)?;
    rig.engine.world.link_exit(rig.exit, Some(rig.source))?;
    rig.engine.world.take_dirty();
    let prepared = rig.prepare("n")?;
    let writes = prepared.0.authoritative_replay_writes();
    check!(
        !writes
            .iter()
            .any(|write| matches!(write, ReplayWrite::ObjectSet { .. }))
    );
    check!(
        writes
            .iter()
            .any(|write| matches!(write, ReplayWrite::LedgerChunkSet { .. }))
    );
    rig.engine.abort_prepared_transaction(prepared.0);
    rig.assert_unpublished()?;
    Ok(())
}

#[test]
fn an_unlocated_actor_can_prepare_home_without_inventing_a_source_image() -> TestResult {
    let mut rig = Rig::new(ObjKind::Npc)?;
    rig.engine
        .world
        .get_mut(rig.actor)
        .ok_or("actor absent")?
        .location = None;
    rig.engine
        .world
        .get_mut(rig.source)
        .ok_or("source absent")?
        .contents
        .retain(|id| *id != rig.actor);
    rig.engine.world.take_dirty();
    let prepared = rig.prepare("home")?;
    let objects: BTreeSet<_> = prepared
        .0
        .authoritative_replay_writes()
        .iter()
        .filter_map(|write| match write {
            ReplayWrite::ObjectSet { object } => Some(object.id),
            _ => None,
        })
        .collect();
    check_eq!(objects, BTreeSet::from([rig.actor, rig.destination]));
    check_eq!(
        rig.engine
            .world
            .get(rig.actor)
            .and_then(|actor| actor.location),
        None
    );
    rig.engine.abort_prepared_transaction(prepared.0);
    check!(!rig.engine.action_state().has_prepared());
    Ok(())
}

#[test]
fn unrelated_window_refuses_initial_preparation_and_late_take() -> TestResult {
    let mut initial = Rig::new(ObjKind::Player)?;
    initial.add_window()?;
    check!(crate::test_checks::refused(initial.prepare("n"))?);
    initial.assert_unpublished()?;
    check!(!initial.engine.action_state().has_prepared());

    let mut late = Rig::new(ObjKind::Player)?;
    let prepared = late.prepare("n")?;
    late.stage(prepared)?;
    late.add_window()?;
    check!(late.engine.take_staged_action().is_err());
    late.assert_unpublished()?;
    check!(!late.engine.action_state().has_prepared());
    Ok(())
}

#[test]
fn callback_free_bind_policy_rejects_a_new_window_outside_the_route_fence() -> TestResult {
    let mut rig = Rig::new(ObjKind::Player)?;
    let fence = StateFence::capture(
        crate::state(&rig.engine.world).ok_or("Sim state absent")?,
        BTreeSet::from([rig.actor, rig.source, rig.destination]),
    )?;
    rig.add_window()?;
    check!(
        fence
            .recheck(crate::state(&rig.engine.world).ok_or("Sim state absent")?)
            .is_err()
    );
    Ok(())
}

#[test]
fn concurrent_recovery_append_refuses_final_take_without_partial_movement() -> TestResult {
    let mut rig = Rig::new(ObjKind::Player)?;
    let prepared = rig.prepare("n")?;
    rig.stage(prepared)?;
    let state = rig
        .engine
        .world
        .module_state_mut::<SpatialState>()
        .ok_or("Sim state absent")?;
    let audit = state
        .recovery
        .prepare_append(RECOVERY_LOG, json!({"op":"fixture_intervening"}))?;
    state.recovery.apply_prepared(audit)?;
    check!(rig.engine.take_staged_action().is_err());
    check_eq!(
        rig.engine
            .world
            .get(rig.actor)
            .and_then(|actor| actor.location),
        Some(rig.source)
    );
    check_eq!(
        crate::state(&rig.engine.world)
            .ok_or("Sim state absent")?
            .recovery
            .next_seq(RECOVERY_LOG),
        1
    );
    check!(!rig.engine.action_state().has_prepared());
    Ok(())
}

#[test]
fn exposed_attached_npc_is_not_silently_left_outside_the_covered_move() -> TestResult {
    for nested in [false, true] {
        let mut rig = Rig::new(ObjKind::Player)?;
        let host = if nested {
            rig.engine
                .world
                .create(ObjKind::Thing, "carried hull", GOD, Some(rig.actor))?
        } else {
            rig.actor
        };
        let npc_location = rig.engine.world.get(host).and_then(|host| host.location);
        let npc = rig
            .engine
            .world
            .create(ObjKind::Npc, "exposed rider", GOD, npc_location)?;
        let mount = MountId::new("exposed").ok_or("mount id invalid")?;
        let state = rig
            .engine
            .world
            .module_state_mut::<SpatialState>()
            .ok_or("Sim state absent")?;
        state.mounts.insert(
            host,
            HostMounts {
                host,
                mounts: BTreeMap::from([(
                    mount.clone(),
                    MountSpec {
                        capacity: CapacitySpec::default(),
                        exposed: true,
                    },
                )]),
            },
        );
        state.placements.insert(
            npc,
            PlacementRecord {
                object: npc,
                parent: SpatialParent::Attached {
                    host,
                    mount,
                    local_pose: None,
                },
                revision: 1,
                unplaced: None,
            },
        );
        state.derived = crate::graph::rebuild(state);
        rig.engine.world.take_dirty();
        check!(crate::test_checks::refused(rig.prepare("n"))?);
        rig.assert_unpublished()?;
        check!(!rig.engine.action_state().has_prepared());
    }
    Ok(())
}

#[test]
fn canonical_suppressed_child_outside_actual_contents_refuses() -> TestResult {
    let mut rig = Rig::new(ObjKind::Player)?;
    let npc = rig
        .engine
        .world
        .create(ObjKind::Npc, "misplaced rider", GOD, Some(rig.source))?;
    let mount = MountId::new("suppressed").ok_or("mount id invalid")?;
    let state = rig
        .engine
        .world
        .module_state_mut::<SpatialState>()
        .ok_or("Sim state absent")?;
    state.mounts.insert(
        rig.actor,
        HostMounts {
            host: rig.actor,
            mounts: BTreeMap::from([(
                mount.clone(),
                MountSpec {
                    capacity: CapacitySpec::default(),
                    exposed: false,
                },
            )]),
        },
    );
    state.placements.insert(
        npc,
        PlacementRecord {
            object: npc,
            parent: SpatialParent::Attached {
                host: rig.actor,
                mount,
                local_pose: None,
            },
            revision: 1,
            unplaced: None,
        },
    );
    state.derived = crate::graph::rebuild(state);
    rig.engine.world.take_dirty();
    check!(crate::test_checks::refused(rig.prepare("n"))?);
    rig.assert_unpublished()?;
    check!(!rig.engine.action_state().has_prepared());
    Ok(())
}
