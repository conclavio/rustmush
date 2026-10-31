#![cfg(test)]

use super::*;
use mush_core::read_service::{OwnerReadError, OwnerReadHandle};
use mush_core::{HookReadContext, Inbound, QueryHandler};
use mush_sim::movement_custody::{MovementCustodyFacts, MovementCustodyRequest};
use mush_tactics::movement_custody::MovementCustodyReader;
use std::sync::{Arc, Mutex};

type Answers = Arc<Mutex<Vec<Result<MovementCustodyFacts, OwnerReadError>>>>;
struct ReadQuery {
    handle: OwnerReadHandle<MovementCustodyReader>,
    answers: Answers,
}
impl QueryHandler for ReadQuery {
    fn query(
        &self,
        _: &mush_core::World,
        _: ObjId,
        _: &Value,
    ) -> Result<OutputEvent, mush_proto::RichText> {
        Err(mush_proto::RichText::plain("actual context required"))
    }
    fn query_with_context(
        &self,
        ctx: &HookReadContext<'_>,
        _: ObjId,
        args: &Value,
    ) -> Result<OutputEvent, mush_proto::RichText> {
        let objects: Vec<u32> = serde_json::from_value(args.clone())
            .map_err(|_| mush_proto::RichText::plain("bad test input"))?;
        let result = ctx.read_owner(
            &self.handle,
            MovementCustodyRequest {
                objects: objects.into_iter().map(ObjId).collect(),
            },
        );
        self.answers
            .lock()
            .map_err(|_| mush_proto::RichText::plain("test lock"))?
            .push(result);
        Ok(OutputEvent::Sys(mush_proto::RichText::plain("read")))
    }
}
/// The read grants only Sim, so the query module stands in under Sim's name.
/// It requests the actual Tactics service; no Sim state or admission exists.
struct SimRequester {
    answers: Answers,
}
impl GameModule for SimRequester {
    fn name(&self) -> &'static str {
        "sim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let handle = MovementCustodyReader::request(reg)?;
        reg.query_handler(
            "test.movement_custody",
            Arc::new(ReadQuery {
                handle,
                answers: self.answers.clone(),
            }),
        )
    }
}
fn fixture() -> (TestWorld, Answers) {
    let answers = Answers::default();
    let world = TestWorld::with_modules(vec![
        Box::new(TestRulesModule {
            owner: "tacsim",
            specs: vec![example_spec(), kitted_spec()],
        }),
        Box::new(SimRequester {
            answers: answers.clone(),
        }),
        Box::new(TacticsModule::with_catalog(example_catalog()).with_movement_custody_reads()),
    ]);
    (world, answers)
}
fn read(
    world: &mut TestWorld,
    answers: &Answers,
    objects: &[ObjId],
) -> Result<MovementCustodyFacts, OwnerReadError> {
    world.engine_mut().step(
        Inbound::Query {
            conn: 1,
            actor: GOD,
            id: 1,
            name: "test.movement_custody".into(),
            args: json!(objects),
        },
        &mut Outbox::default(),
        &mut Effects::default(),
    );
    answers
        .lock()
        .map_err(|_| OwnerReadError::Integrity)?
        .pop()
        .ok_or(OwnerReadError::Unavailable)?
}

#[test]
fn actual_sim_query_reads_tactics_marker_even_with_an_empty_sim_projection() {
    let (mut world, answers) = fixture();
    let unit = make_unit(&mut world);
    let initial = read(&mut world, &answers, &[unit, GOD]).expect("owner read");
    let actual = initial.facts.iter().find(|f| f.object == unit).unwrap();
    assert!(actual.active_unit);
    assert!(!actual.held_record && !actual.ooc_deployed && !actual.ic_custodied);
    let snapshot = store(&world).units[&unit].clone();
    // Fault injection changes only the real Tactics marker map. It does not
    // perform a checkout or create a deployment/physical authority fixture.
    world
        .world_mut()
        .module_state_mut::<UnitStore>()
        .unwrap()
        .deployed
        .insert(
            unit,
            mush_tactics::custody::DeployedMarker {
                mark: mush_sim::DeployedMark::new(7, "arena"),
                domain: None,
                pose: None,
                snapshot,
            },
        );
    assert!(!mush_sim::deployed::is_deployed(world.world(), unit));
    let changed = read(&mut world, &answers, &[unit, GOD]).expect("actual Tactics source");
    assert!(
        changed
            .facts
            .iter()
            .find(|f| f.object == unit)
            .unwrap()
            .ooc_deployed
    );
    assert_ne!(initial.digest, changed.digest);
    assert_eq!(read(&mut world, &answers, &[]), Err(OwnerReadError::Limit));
    let excess: Vec<_> = (0..513).map(ObjId).collect();
    assert_eq!(
        read(&mut world, &answers, &excess),
        Err(OwnerReadError::Limit)
    );
}

#[test]
fn real_frozen_unit_records_remain_held_and_a_custody_read_publishes_nothing() {
    let (mut world, answers) = fixture();
    let unit = world
        .world_mut()
        .create(ObjKind::Thing, "missing binding", GOD, None)
        .unwrap();
    let cells = TacticalUnit::from_spec(&example_spec(), unit).unwrap();
    let document = current_document(&world, &cells);
    load_tactics(
        &mut world,
        vec![(SmolStr::new(format!("unit:{}", unit.0)), document)],
    );
    assert!(matches!(
        store(&world).records[0].state,
        MechanicalState::Frozen { .. }
    ));
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    let rows = world.engine().action_state().len();
    let result = read(&mut world, &answers, &[unit, GOD]).expect("held lifecycle observation");
    let actual = result.facts.iter().find(|f| f.object == unit).unwrap();
    assert!(!actual.active_unit && actual.held_record);
    assert!(world.world_mut().take_dirty().is_empty());
    assert!(world.take_module_ops().is_empty());
    assert_eq!(world.engine().action_state().len(), rows);
}

#[test]
fn custody_reads_bound_the_global_record_scan_and_selected_key_text() {
    let (mut world, answers) = fixture();
    // Metadata-only fault injection exercises scan limits without constructing
    // thousands of opaque units or claiming any lifecycle/custody authority.
    let row = mush_tactics::module::UnitRecordStatus {
        key: "unselected".into(),
        key_object: None,
        object: None,
        state: MechanicalState::Active,
        unit: None,
        payload_digest: None,
        migration_plan: None,
        deployed: None,
    };
    world
        .world_mut()
        .module_state_mut::<UnitStore>()
        .unwrap()
        .records = vec![row.clone(); 4096];
    assert!(read(&mut world, &answers, &[GOD]).is_ok());
    world
        .world_mut()
        .module_state_mut::<UnitStore>()
        .unwrap()
        .records
        .push(row.clone());
    assert_eq!(
        read(&mut world, &answers, &[GOD]),
        Err(OwnerReadError::Limit)
    );
    let mut oversized = row;
    oversized.key = "x".repeat(1025).into();
    oversized.key_object = Some(GOD);
    world
        .world_mut()
        .module_state_mut::<UnitStore>()
        .unwrap()
        .records = vec![oversized];
    assert_eq!(
        read(&mut world, &answers, &[GOD]),
        Err(OwnerReadError::Limit)
    );
}
