//! Private runtime capacity, using actual Engine-minted observations. These
//! selectors test volatile custody only; they are not funded purchase fixtures.
use super::*;
use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind};
use mush_core::submission_observation::{
    SubmissionDisposition, SubmissionDispositionReader, SubmissionRuntime, SubmissionRuntimeState,
};
use mush_core::{Engine, GOD, GameModule, Inbound, Outbox, Registry, World};
use std::sync::atomic::{AtomicBool, Ordering};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<SubmissionObservation>>>);
struct Fixture(Captured);
impl GameModule for Fixture {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let descriptor = DefinitionDescriptor::new(
            DefinitionKind::new("attempt_fixture").map_err(super::super::error)?,
            DefinitionId::new("capacity").map_err(super::super::error)?,
            1,
            serde_json::json!({"scope":"volatile observation custody only"}),
        )
        .map_err(super::super::error)?;
        reg.definition_validator(
            descriptor.kind().clone(),
            crate::lot::exact_validator(&[descriptor.clone()]),
        )?;
        let action = reg.definition(descriptor)?;
        let capture = self.0.clone();
        reg.classify_commands(CommandCompatibility::Authoritative);
        reg.command(
            CommandSpec {
                name: "@capture-purchase-attempt".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let observation = ctx.submit_authoritative_observed(
                    action.clone(),
                    PerceivedRef::SelfActor,
                    vec![],
                    serde_json::json!({}),
                    IdempotencyKeyText::parse_canonical("capacity.fixture")
                        .map_err(|_| unavailable())?,
                )?;
                capture
                    .0
                    .lock()
                    .map_err(|_| unavailable())?
                    .push(observation);
                Ok(())
            }),
        )
    }
}
struct Reader(Arc<AtomicBool>);
impl SubmissionDispositionReader for Reader {
    fn no_intent(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
struct ProofRuntime;
impl SubmissionRuntime for ProofRuntime {
    fn observe(&self, _: &mush_core::AuthoritativeSubmit) -> SubmissionRuntimeState {
        let proof = Arc::new(AtomicBool::new(false));
        SubmissionRuntimeState::new(Arc::new(Reader(proof.clone())), Box::new(proof))
    }
}
fn capture(
    engine: &mut Engine,
    captured: &Captured,
) -> TestResult<(DefinitionRef, SubmissionObservation, Arc<AtomicBool>)> {
    engine.step(
        Inbound::Cmd {
            conn: 0,
            actor: GOD,
            line: "@capture-purchase-attempt".into(),
        },
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    );
    let observation = captured
        .0
        .lock()
        .map_err(|_| "capture mutex")?
        .pop()
        .ok_or("actual observation")?;
    let driven = engine
        .take_observed_authoritative_submit()
        .ok_or("actual queue handoff")?;
    check!(driven.is_observed());
    let (selector, driver) = driven.into_parts();
    let proof = *driver
        .ok_or("actual runtime state")?
        .downcast::<Arc<AtomicBool>>()
        .map_err(|_| "proof type")?;
    Ok((selector.action, observation, proof))
}
fn operation(n: u128) -> CommitToken {
    CommitToken::from_bytes(n.to_be_bytes())
}

#[test]
fn sixty_four_unresolved_attempts_refuse_the_next_without_replacing_any_observation() -> TestResult
{
    let captured = Captured::default();
    let mut engine = Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![Box::new(Fixture(captured.clone()))],
    )?;
    engine.set_submission_runtime(Arc::new(ProofRuntime))?;
    let mut runtime = Runtime::default();
    let mut proofs = BTreeMap::new();
    for n in 1..=MAX_ATTEMPTS {
        let id = operation(u128::try_from(n)?);
        let entry = runtime.admit(id, GOD)?;
        let (action, observation, proof) = capture(&mut engine, &captured)?;
        entry.schedule_after = i64::try_from(n)?;
        entry.attempt = Some((action, observation));
        proofs.insert(id, proof);
    }
    let snapshot = runtime
        .entries
        .iter()
        .map(|(id, entry)| {
            Ok((
                *id,
                entry.buyer,
                entry.schedule_after,
                entry.attempt.as_ref().ok_or("held attempt")?.0.clone(),
            ))
        })
        .collect::<TestResult<Vec<_>>>()?;
    check_eq!(runtime.entries.len(), 64);
    for _ in 0..3 {
        check!(runtime.admit(operation(65), GOD).is_err());
        check!(!runtime.entries.contains_key(&operation(65)));
        check_eq!(runtime.entries.len(), 64);
    }
    check!(runtime.admit(operation(1), ObjId(2)).is_err());
    check!(
        runtime.admit(operation(1), GOD)?.attempt.is_some(),
        "existing custody survives saturation"
    );
    for (id, buyer, after, action) in &snapshot {
        let entry = runtime.entries.get_mut(id).ok_or("retained entry")?;
        check_eq!(&entry.buyer, buyer);
        check_eq!(&entry.schedule_after, after);
        let (actual, observation) = entry.attempt.as_mut().ok_or("retained observation")?;
        check_eq!(&*actual, action);
        check_eq!(observation.take_terminal(), None);
    }
    // Independently completing one exact backend proves the preserved handle
    // was not silently replaced, duplicated or borrowed from another entry.
    let selected = operation(32);
    proofs
        .get(&selected)
        .ok_or("exact proof")?
        .store(true, Ordering::Release);
    for (id, entry) in &mut runtime.entries {
        let (_, observation) = entry.attempt.as_mut().ok_or("held observation")?;
        check_eq!(
            observation.take_terminal(),
            if *id == selected {
                Some(SubmissionDisposition::NoIntent)
            } else {
                None
            }
        );
        check_eq!(observation.take_terminal(), None);
    }
    check_eq!(runtime.entries.len(), 64);
    check!(
        runtime.admit(operation(65), GOD).is_err(),
        "a terminal observation alone does not evict custody"
    );
    Ok(())
}
