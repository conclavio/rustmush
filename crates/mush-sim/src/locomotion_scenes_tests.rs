#![cfg(test)]

use super::*;
use mush_core::action::{
    ActionInvocationDraft, ActionOfferId, CanonicalHash, CommitToken, IdempotencyKey,
    IdempotencyScope, PreparedAuthoritativeAction, RequestUid, ScopedIdempotencyKey,
};
use mush_core::composed::PreparedComposedTxn;
use mush_core::{
    CanonicalId, DefinitionKey, Effects, Engine, Flags, GOD, ObjId, ObjKind, Outbox, PerceivedRef,
    PrincipalId,
};
use mush_scenes::{ScenesModule, ScenesState};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);

struct Rig {
    engine: Box<Engine>,
    action: DefinitionRef,
    actor: ObjId,
    source: ObjId,
    destination: ObjId,
    scene_logs: Vec<(String, u64)>,
}

impl Rig {
    fn new(kind: ObjKind) -> TestResult<Self> {
        Self::with_scenes(kind, true)
    }

    fn with_scenes(kind: ObjKind, with_scenes: bool) -> TestResult<Self> {
        let reader = Arc::new(OnceLock::new());
        let mut test = mush_testkit::TestWorld::with_modules(vec![
            Box::new(crate::SimulationModule::new().with_atomic_scene_movement(reader.clone())),
            Box::new(ScenesModule::new().with_atomic_movement(reader)),
        ]);
        test.set_now(100);
        let source = test
            .world_mut()
            .create(ObjKind::Room, "source", GOD, None)?;
        let destination = test
            .world_mut()
            .create(ObjKind::Room, "destination", GOD, None)?;
        let actor = test.world_mut().create(kind, "walker", GOD, Some(source))?;
        // Real Timestops freeze their participants. Staff movement is the
        // existing owner-approved exception for either Player or NPC walkers.
        test.world_mut()
            .get_mut(actor)
            .ok_or("actor absent")?
            .flags
            .insert(Flags::WIZARD);
        if kind == ObjKind::Player {
            let _ = test.connect(actor);
        }
        let exit = test
            .world_mut()
            .create(ObjKind::Exit, "north;n", GOD, Some(source))?;
        test.world_mut().link_exit(exit, Some(destination))?;
        test.world_mut().set_home(actor, Some(destination))?;
        let mut scene_logs = Vec::new();
        if with_scenes {
            for (name, room) in [
                ("source facilitator", source),
                ("destination facilitator", destination),
            ] {
                let host = test
                    .world_mut()
                    .create(ObjKind::Player, name, GOD, Some(room))?;
                let _ = test.connect(host);
                let _ = test.run(host, "+timestop");
                let state = test
                    .world()
                    .module_state::<ScenesState>()
                    .ok_or("Scenes absent")?;
                let log = state
                    .active_scene_at(room)
                    .ok_or("Timestop did not start")?
                    .id
                    .to_string();
                let next = state.events.next_seq(&log);
                check_eq!(next, 1); // The real command records Started.
                scene_logs.push((log, next));
            }
        }
        let _ = test.take_module_ops();
        test.world_mut().take_dirty();
        let engine = Box::new(test.into_engine());
        let action = engine
            .registry
            .definitions()
            .ok_or("definitions absent")?
            .current(&DefinitionKey {
                owner: OwnerId::new("sim")?,
                kind: DefinitionKind::new("action")?,
                id: DefinitionId::new(scene_transition::ACTION)?,
            })
            .cloned()
            .ok_or("joint action absent")?;
        Ok(Self {
            engine,
            action,
            actor,
            source,
            destination,
            scene_logs,
        })
    }

    fn invocation(&self) -> TestResult<ActionInvocation> {
        self.invocation_with(json!({}))
    }

    fn invocation_with(&self, parameters: serde_json::Value) -> TestResult<ActionInvocation> {
        let request = RequestUid::from_bytes([93; 16]);
        let principal = PrincipalId::Object { id: self.actor };
        Ok(ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: self.actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([93; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("sim")?,
                    namespace: CanonicalId::new("scene_locomotion_test")?,
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
            .ok_or("no captured route")?;
        check!(out.is_empty());
        Ok(self.engine.prepare_captured_locomotion_action(
            captured,
            &self.action,
            &self.invocation()?,
            CommitToken::from_bytes([93; 16]),
        )?)
    }

    fn unpublished(&self) -> TestResult {
        check_eq!(
            self.engine
                .world
                .get(self.actor)
                .and_then(|object| object.location),
            Some(self.source)
        );
        check_eq!(
            crate::state(&self.engine.world)
                .ok_or("Sim absent")?
                .recovery
                .next_seq(RECOVERY_LOG),
            0
        );
        let scenes = self
            .engine
            .world
            .module_state::<ScenesState>()
            .ok_or("Scenes absent")?;
        for (log, next) in &self.scene_logs {
            check_eq!(scenes.events.next_seq(log), *next);
        }
        if self.scene_logs.is_empty() {
            check!(scenes.scenes.is_empty());
        }
        check!(!self.engine.has_legacy_persistence_work());
        Ok(())
    }
}

#[test]
fn captured_player_and_npc_prepare_both_ledgers_and_only_joint_families() -> TestResult {
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
        let logs: BTreeSet<_> = writes
            .iter()
            .filter_map(|write| match write {
                ReplayWrite::LedgerChunkSet { write } => Some((
                    write.namespace().as_str().to_owned(),
                    write.log_id().to_owned(),
                )),
                _ => None,
            })
            .collect();
        let expected_logs: BTreeSet<_> =
            std::iter::once(("sim".to_owned(), RECOVERY_LOG.to_owned()))
                .chain(
                    rig.scene_logs
                        .iter()
                        .map(|(log, _)| ("scenes".to_owned(), log.clone())),
                )
                .collect();
        check_eq!(logs, expected_logs);
        let typed: Vec<_> = writes
            .iter()
            .filter_map(|write| match write {
                ReplayWrite::ModuleJsonSet { key, .. } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        check!(
            typed
                .iter()
                .any(|key| key.starts_with("action-state/v1/locomotion_scene_transitions/"))
        );
        check!(
            typed
                .iter()
                .any(|key| key.starts_with("action-state/v1/locomotion_scene_crossings/"))
        );
        check!(!typed.iter().any(
            |key| key.starts_with("action-state/v1/locomotion_transitions/")
                || key.starts_with("action-state/v1/locomotion_crossings/")
        ));
        rig.unpublished()?;
        rig.engine.abort_prepared_transaction(prepared.0);
        check!(!rig.engine.action_state().has_prepared());
        rig.unpublished()?;
    }
    Ok(())
}

#[test]
fn focus_presence_drift_refuses_both_native_and_scene_publication() -> TestResult {
    let mut rig = Rig::new(ObjKind::Player)?;
    let unit =
        rig.engine
            .world
            .create(ObjKind::Thing, "other unit", GOD, Some(mush_core::LIMBO))?;
    rig.engine.world.take_dirty();
    let prepared = rig.prepare("north")?;
    let revision = rig.engine.action_state().revision();
    let records = rig.engine.action_state().len();
    let (_, install) = prepared.0.into_parts();
    // Seed only the actual exclusion predicate. This is not a typed lease or
    // proxy-seat authority, and no SpatialState generation is changed.
    let focus = crate::ActiveFocus {
        account: mush_core::AccountId::mint("focus_test", 1)?,
        actor: rig.actor,
        driver: rig.actor,
        unit,
        station: crate::StationId::new("pilot").ok_or("station")?,
        acquired_at: rig.engine.world.world_ic_now(),
        epoch: 1,
        reach: crate::ReachProof::Console { room: rig.source },
        proxy: false,
    };
    rig.engine
        .world
        .module_state_mut::<crate::FocusState>()
        .ok_or("focus")?
        .claim(focus);
    check!(rig.engine.install_composed_action(install).is_err());
    rig.unpublished()?;
    check_eq!(rig.engine.action_state().revision(), revision);
    check_eq!(rig.engine.action_state().len(), records);
    check!(rig.engine.action_state().reservation_lost());
    Ok(())
}

#[test]
fn joint_provider_requires_actual_capture() -> TestResult {
    let mut rig = Rig::new(ObjKind::Player)?;
    check!(
        rig.engine
            .prepare_registered_composed_authoritative_action(
                &rig.action,
                &rig.invocation()?,
                CommitToken::from_bytes([93; 16])
            )
            .is_err()
    );
    rig.unpublished()?;
    check!(!rig.engine.action_state().has_prepared());
    let mut out = Outbox::default();
    let captured = rig
        .engine
        .capture_locomotion_command(rig.actor, "n", &mut out, &mut Effects::default())
        .ok_or("no capture")?;
    let forged = rig.invocation_with(json!({"destination":rig.source}))?;
    check!(
        rig.engine
            .prepare_captured_locomotion_action(
                captured,
                &rig.action,
                &forged,
                CommitToken::from_bytes([93; 16])
            )
            .is_err()
    );
    check!(!rig.engine.action_state().has_prepared());
    rig.unpublished()?;
    Ok(())
}

#[test]
fn empty_scene_capture_retains_the_joint_receipt_without_a_synthetic_event() -> TestResult {
    let mut rig = Rig::with_scenes(ObjKind::Player, false)?;
    let prepared = rig.prepare("n")?;
    let writes = prepared.0.authoritative_replay_writes();
    check!(!writes.iter().any(
        |write| matches!(write, ReplayWrite::LedgerChunkSet { write }
        if write.namespace().as_str() == "scenes")
    ));
    check!(writes.iter().any(
        |write| matches!(write, ReplayWrite::ModuleJsonSet { key, .. }
        if key.as_str().starts_with("action-state/v1/locomotion_scene_transitions/"))
    ));
    rig.unpublished()?;
    rig.engine.abort_prepared_transaction(prepared.0);
    check!(!rig.engine.action_state().has_prepared());
    Ok(())
}

#[test]
fn default_composition_retains_only_the_original_locomotion_action() -> TestResult {
    let test =
        mush_testkit::TestWorld::with_modules(vec![Box::new(crate::SimulationModule::new())]);
    let definitions = test
        .engine()
        .registry
        .definitions()
        .ok_or("definitions absent")?;
    let key = |id| -> TestResult<DefinitionKey> {
        Ok(DefinitionKey {
            owner: OwnerId::new("sim")?,
            kind: DefinitionKind::new("action")?,
            id: DefinitionId::new(id)?,
        })
    };
    check!(
        definitions
            .current(&key(mush_core::locomotion::transition::ACTION)?)
            .is_some()
    );
    check!(
        definitions
            .current(&key(scene_transition::ACTION)?)
            .is_none()
    );
    Ok(())
}
