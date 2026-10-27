#![cfg(test)]

//! Actual Registrar/Engine capacity boundary. Invocation budgets stay covered by
//! the existing read/scan/request/result exhaustion tests in the parent module.
use super::*;

#[derive(Clone)]
struct QuotaSlot(u32);
struct QuotaService<const N: usize>;
impl<const N: usize> OwnerReadService for QuotaService<N> {
    type State = QuotaSlot;
    type Request = ();
    type Output = (usize, ObjId, String, u32);
    fn read(
        &self,
        state: &QuotaSlot,
        ctx: &OwnerReadContext<'_>,
        _: (),
    ) -> Result<Self::Output, OwnerReadError> {
        Ok((N, ctx.actor(), ctx.requester().as_str().to_owned(), state.0))
    }
}
fn quota_descriptor(n: usize) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").unwrap(),
        DefinitionId::new(format!("service_{n}")).unwrap(),
        1,
        json!({"format":1,"service":n}),
    )
    .unwrap()
}
fn quota_definition(n: usize) -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new("quota_owner").unwrap(), quota_descriptor(n))
        .unwrap()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Good,
    Overflow,
    DuplicateType,
    DuplicateDefinition,
    ForeignOwner,
    WrongType,
    Denied,
}
struct QuotaOwner(Mode);
fn register_one<const N: usize>(
    reg: &mut Registrar<'_>,
    n: usize,
    allow: bool,
) -> Result<(), ModError> {
    let definition = reg.definition(quota_descriptor(n))?;
    reg.owner_read_service(
        definition,
        if allow {
            BTreeSet::from([OwnerId::new("quota_consumer").unwrap()])
        } else {
            BTreeSet::new()
        },
        BTreeSet::new(),
        QuotaService::<N>,
    )
}
impl GameModule for QuotaOwner {
    fn name(&self) -> &'static str {
        "quota_owner"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(QuotaSlot(73));
        reg.definition_validator(
            DefinitionKind::new("owner_read").unwrap(),
            |ctx: &crate::mechanics::DefinitionValidationContext<'_>| {
                let entry = ctx.definition();
                if (0..=64).any(|n| {
                    entry.reference() == &quota_definition(n)
                        && entry.payload() == quota_descriptor(n).payload()
                        && entry.references().is_empty()
                        && !entry.is_current()
                }) {
                    Ok(())
                } else {
                    Err("quota fixture definition differs".into())
                }
            },
        )?;
        macro_rules! register {($($n:literal),*)=>{$(register_one::<$n>(reg,$n,true)?;)*};}
        register!(
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
            46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62
        );
        match self.0 {
            Mode::DuplicateType => register_one::<0>(reg, 63, true),
            Mode::DuplicateDefinition => reg.owner_read_service(
                quota_definition(0),
                BTreeSet::new(),
                BTreeSet::new(),
                QuotaService::<63>,
            ),
            Mode::ForeignOwner => {
                let definition = DefinitionRegistryBuilder::new()
                    .register(OwnerId::new("foreign").unwrap(), quota_descriptor(63))
                    .unwrap();
                reg.owner_read_service(
                    definition,
                    BTreeSet::new(),
                    BTreeSet::new(),
                    QuotaService::<63>,
                )
            }
            mode => {
                register_one::<63>(reg, 63, mode != Mode::Denied)?;
                if mode == Mode::Overflow {
                    register_one::<64>(reg, 64, true)?;
                }
                Ok(())
            }
        }
    }
}
type Answers = Arc<Mutex<Vec<(usize, ObjId, String, u32)>>>;
struct QuotaConsumer {
    mode: Mode,
    answers: Answers,
}
fn request_one<const N: usize>(
    reg: &mut Registrar<'_>,
    id: usize,
    answers: &Answers,
) -> Result<(), ModError> {
    let handle = reg.request_owner_read::<QuotaService<N>>(quota_definition(id))?;
    let answers = Arc::clone(answers);
    reg.command(
        CommandSpec {
            name: format!("quotaread{id}").into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| {
            let actual = ctx
                .read_owner(&handle, ())
                .map_err(|e| CmdError::msg(e.to_string()))?;
            answers.lock().unwrap().push(actual);
            Ok(())
        }),
    )
}
impl GameModule for QuotaConsumer {
    fn name(&self) -> &'static str {
        "quota_consumer"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(crate::CommandCompatibility::ReadOnly);
        macro_rules! request {($($n:literal),*)=>{$(request_one::<$n>(reg,$n,&self.answers)?;)*};}
        request!(
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
            46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62
        );
        if self.mode == Mode::WrongType {
            request_one::<64>(reg, 63, &self.answers)
        } else {
            request_one::<63>(reg, 63, &self.answers)
        }
    }
}
fn quota_engine(mode: Mode, reversed: bool) -> Result<(Engine, Answers), ModError> {
    let answers = Arc::new(Mutex::new(Vec::new()));
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(QuotaOwner(mode)),
        Box::new(QuotaConsumer {
            mode,
            answers: Arc::clone(&answers),
        }),
    ];
    if reversed {
        modules.reverse();
    }
    Ok((
        Engine::with_modules(World::new_minimal(), Registry::new(), modules)?,
        answers,
    ))
}

#[test]
fn sixty_four_distinct_registered_services_dispatch_and_sixty_fifth_refuses() {
    for reversed in [false, true] {
        let (mut engine, answers) = quota_engine(Mode::Good, reversed).unwrap();
        for n in 0..64 {
            engine.step(
                Inbound::Cmd {
                    conn: 1,
                    actor: GOD,
                    line: format!("quotaread{n}"),
                },
                &mut Outbox::new(),
                &mut crate::engine::Effects::new(),
            );
        }
        let answers = answers.lock().unwrap();
        assert_eq!(answers.len(), 64);
        for (n, (service, actor, requester, value)) in answers.iter().enumerate() {
            assert_eq!(
                (*service, *actor, requester.as_str(), *value),
                (n, GOD, "quota_consumer", 73)
            );
        }
        assert!(engine.action_state().is_empty());
        assert!(!engine.action_state().has_prepared());
        let error = quota_engine(Mode::Overflow, reversed)
            .err()
            .expect("65 distinct declarations must refuse");
        assert!(
            error
                .to_string()
                .contains("owner read exceeded its bounded envelope")
        );
    }
}

#[test]
fn expanded_declarations_keep_exact_definition_type_and_requester_authority() {
    for mode in [
        Mode::DuplicateType,
        Mode::DuplicateDefinition,
        Mode::ForeignOwner,
        Mode::WrongType,
        Mode::Denied,
    ] {
        let error = quota_engine(mode, false)
            .err()
            .expect("crossed service authority must refuse");
        assert!(
            error
                .to_string()
                .contains("owner read registration or caller is invalid"),
            "wrong refusal layer for {mode:?}: {error}"
        );
    }
}
