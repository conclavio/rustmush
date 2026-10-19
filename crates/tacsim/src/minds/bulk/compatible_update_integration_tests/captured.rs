//! Real pre-update checkout retained across the actual new line publication.
use super::*;
pub(super) fn apply(
    world: &mut PlayerWorld,
    writes: &[ReplayWrite],
    install: mush_core::composed::ComposedInstallation,
) -> TestResult {
    let operations = writes
        .iter()
        .map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => Ok(Some(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: key.to_string().into(),
                    value: value.clone(),
                },
            })),
            ReplayWrite::ModuleDelete { namespace, key } => Ok(Some(ModuleOp {
                ns: namespace.to_string().into(),
                op: ModuleWrite::Del {
                    key: key.to_string().into(),
                },
            })),
            ReplayWrite::LedgerChunkSet { write } => Ok(Some(ModuleOp {
                ns: write.namespace().to_string().into(),
                op: ModuleWrite::JsonSet {
                    key: write.key().to_string().into(),
                    value: serde_json::to_value(write.chunk())?,
                },
            })),
            ReplayWrite::ObjectSet { .. } | ReplayWrite::ObjectAllocatorSet { .. } => Ok(None),
            ReplayWrite::ObjectDelete { .. } => Err("filing does not delete objects".into()),
        })
        .collect::<TestResult<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    mush_testkit::fold_module_ops_into(&mut world.image, operations);
    world.engine.install_composed_action(install)?;
    world.commit()?;
    Ok(())
}
pub(super) async fn exercise(
    f: &PurchaseFixture,
    saved: &Durable,
    revision: &Revision,
    rating: RatingRef,
) -> TestResult {
    let mut world = PlayerWorld::boot_updates(f, saved)?;
    let tasks = world.tasks.clone();
    let outcome: TestResult = async {
        world.select()?;
        world.start()?;
        world.board()?;
        assert_eq!(world.state()?, MatchState::Running);
        let id = match_::centre_of(&world.engine.world)
            .and_then(|c| c.match_id)
            .ok_or("actual running sortie")?;
        let record = world.record(id)?;
        let unit = record
            .teams
            .iter()
            .flat_map(|s| s.pilots.iter())
            .find_map(|(p, u)| (*p == BUYER).then_some(*u))
            .ok_or("actual buyer hull")?;
        let seal = record
            .minds
            .get(&unit)
            .ok_or("actual old seal")?
            .seal
            .clone();
        assert_eq!(seal.origin.licence_label(), "2");
        world.read_sealed_mind(&seal.mind).await?;
        let deadline = Instant::now() + WAIT;
        while !world.snapshots.contains_key(&id) {
            world.pump()?;
            if Instant::now() >= deadline {
                return Err("opening snapshot guard".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let first = world.snapshots.get(&id).ok_or("opening snapshot")?;
        let ground =
            crate::arena_bridge::ground_of(&world.engine.world, id).ok_or("actual ground")?;
        let arena = Snapshot::decode(&first.document, (*ground).clone())?;
        let pilot = arena
            .units
            .iter()
            .find(|u| u.id == unit)
            .ok_or("opening hull")?;
        assert_eq!(pilot.controller, Controller::Pilot);
        assert_eq!(pilot.seal, seal.seal_ref());
        let action = world
            .engine
            .registry
            .definitions()
            .ok_or("definitions")?
            .iter()
            .map(|e| e.reference())
            .find(|r| {
                r.key().owner.as_str() == "roll"
                    && r.key().kind.as_str() == "action"
                    && r.key().id.as_str() == "update_original_mind"
            })
            .cloned()
            .ok_or("actual update action")?;
        let (prepared, _) = world
            .engine
            .prepare_registered_composed_authoritative_action(
                &action,
                &request(START + 20, GOD, "roll", parameters(revision, rating))?,
                token(START + 20),
            )?;
        let (writes, installation) = prepared.into_parts();
        apply(&mut world, &writes, installation)?;
        let saved = Durable::capture(&world.engine, &disk(&world));
        let new: FiledRecord = serde_json::from_value(
            rows(&saved.disk, "roll", "roll_mind_filed")
                .into_iter()
                .find(|v| v.get("format") == Some(&json!(3)))
                .ok_or("actual update row")?,
        )?;
        assert_eq!(new.content().revision().revision(), revision.reference());
        let old_certificate = seal
            .requested
            .origin
            .certificate()
            .ok_or("old certificate")?;
        assert_ne!(old_certificate.rating_job, rating.job());
        assert_eq!(
            &world
                .record(id)?
                .minds
                .get(&unit)
                .ok_or("retained old seal")?
                .seal,
            &seal
        );
        world.read_sealed_mind(&seal.mind).await?;
        let before = world.snapshots.get(&id).map_or(-1, |s| s.at_ms);
        world.say(BUYER, "EXIT")?;
        let first = world.progressed_snapshot(id, unit, before, 0, -1).await?;
        let (thinks, landed) = assert_captured(&world, id, unit, &seal, &first)?;
        let later = world
            .progressed_snapshot(id, unit, first.at_ms, thinks, landed)
            .await?;
        let (after, after_landed) = assert_captured(&world, id, unit, &seal, &later)?;
        assert!(after > thinks && after_landed > landed);
        Ok(())
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    outcome
}
