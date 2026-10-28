//! Actual Engine declaration/grant validation. The two owner adapters always
//! refuse preparation: this fixture does not create native business evidence.
use super::*;
use crate::mechanics::DefinitionValidationContext;
use crate::participant::{
    BoundParticipant, OwnerParticipant, OwnerPreparation, OwnerPrepareContext, ParticipantError,
};
use crate::{Engine, GameModule, ModError, Registrar, Registry, World};
#[derive(Clone, Default)]
struct Cell<const N: u8>;
struct Adapter<const N: u8>;
struct Never;
impl BoundParticipant for Never {
    fn apply(self) {}
}
impl<const N: u8> OwnerParticipant for Adapter<N> {
    type State = Cell<N>;
    type Request = ();
    type Output = ();
    type Patch = ();
    type Bound<'a> = Never;
    fn prepare(
        &self,
        _: &Cell<N>,
        _: &OwnerPrepareContext<'_>,
        _: (),
    ) -> Result<OwnerPreparation<(), ()>, ParticipantError> {
        Err(ParticipantError::Authority)
    }
    fn recheck(
        &self,
        _: &Cell<N>,
        _: &OwnerPrepareContext<'_>,
        _: &(),
    ) -> Result<(), ParticipantError> {
        Err(ParticipantError::Authority)
    }
    fn bind(self: Arc<Self>, _: &mut Cell<N>, _: ()) -> Result<Never, ParticipantError> {
        Err(ParticipantError::Authority)
    }
    fn accepts_write(&self, _: &crate::ReplayWrite) -> bool {
        false
    }
}
fn me(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn participant_descriptor(n: usize) -> TestResult<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new("owner_participant")?,
        DefinitionId::new(if n == 0 {
            "initial_filing"
        } else {
            "commercial_roots"
        })?,
        1,
        json!({"format":1}),
    )?)
}
fn checked(
    reg: &mut Registrar<'_>,
    kind: &str,
    expected: Vec<DefinitionRef>,
) -> Result<(), ModError> {
    reg.definition_validator(
        DefinitionKind::new(kind).map_err(me)?,
        move |ctx: &DefinitionValidationContext<'_>| {
            if expected.contains(ctx.definition().reference()) {
                Ok(())
            } else {
                Err("exact pending fixture reference".into())
            }
        },
    )
}
/// 17g C2f: how the fixture registers the owner-index family the Offer and
/// Fund write (exactly, not at all, or with a one-byte-smaller record bound).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Index {
    Exact,
    Missing,
    Descriptor,
}
struct Module<const N: usize> {
    protocol: Box<Protocol>,
    missing: Option<usize>,
    bad_family: bool,
    index: Index,
}
impl<const N: usize> GameModule for Module<N> {
    fn name(&self) -> &'static str {
        if N == 0 { "economy" } else { "roll" }
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        checked(
            reg,
            "owner_participant",
            vec![
                self.protocol
                    .participants
                    .get(N)
                    .ok_or_else(|| me("participant"))?
                    .clone(),
            ],
        )?;
        let participant = reg.definition(participant_descriptor(N).map_err(me)?)?;
        let all = self
            .protocol
            .actions
            .iter()
            .chain(self.protocol.registry_revocation.iter())
            .cloned()
            .collect::<Vec<_>>();
        if N == 0 {
            reg.state(Cell::<0>);
            reg.owner_participant(
                participant,
                all.iter().skip(1).cloned().collect(),
                Adapter::<0>,
            )?;
            return Ok(());
        }
        reg.state(Cell::<1>);
        reg.owner_participant(participant, all.iter().cloned().collect(), Adapter::<1>)?;
        for (family, max) in FAMILIES
            .into_iter()
            .chain([(SANCTION, 32768), (STATUS, 4096)])
        {
            let descriptor = protocol::descriptor(
                family,
                if self.bad_family && family == PENDING {
                    max - 1
                } else {
                    max
                },
            )
            .map_err(me)?;
            reg.state_family(
                descriptor,
                |k: &String| Ok(k.clone()),
                |v: &Value| Ok(v.clone()),
                |_: &Value| Ok(()),
                |v: &Value| Ok(v.clone()),
            )?;
        }
        // 17g C2f: the named buyer's offer index the Offer and Fund write.
        if self.index != Index::Missing {
            let (_, mut index, _) =
                owner_index::descriptor(owner_index::PENDING_OFFERS).map_err(me)?;
            if self.index == Index::Descriptor {
                let (key, bytes) = (index.max_key_bytes(), index.max_record_bytes());
                index = index.with_limits(key, bytes - 1).map_err(me)?;
            }
            reg.state_family(
                index,
                |k: &String| Ok(k.clone()),
                |v: &Value| Ok(v.clone()),
                |_: &Value| Ok(()),
                |v: &Value| Ok(v.clone()),
            )?;
        }
        let authority = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("roll").map_err(me)?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("registry_authority").map_err(me)?,
                    DefinitionId::new("mind_revocation").map_err(me)?,
                    1,
                    super::authority(),
                )
                .map_err(me)?,
            )
            .map_err(me)?;
        checked(reg, "registry_authority", vec![authority.clone()])?;
        reg.definition(
            DefinitionDescriptor::new(
                DefinitionKind::new("registry_authority").map_err(me)?,
                DefinitionId::new("mind_revocation").map_err(me)?,
                1,
                super::authority(),
            )
            .map_err(me)?,
        )?;
        checked(reg, "action", all.clone())?;
        for (i, a) in all.into_iter().enumerate() {
            let d = if i < 4 {
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").map_err(me)?,
                    DefinitionId::new(*ACTIONS.get(i).ok_or_else(|| me("action"))?).map_err(me)?,
                    if i == 0 { 2 } else { 1 },
                    protocol::action_payload(i),
                )
                .map_err(me)?
                .with_references(vec![self.protocol.participants[1].clone()])
            } else {
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").map_err(me)?,
                    DefinitionId::new("registry_term_revocation").map_err(me)?,
                    1,
                    protocol::revocation_payload(),
                )
                .map_err(me)?
                .with_references(vec![
                    authority.clone(),
                    self.protocol.participants[1].clone(),
                ])
            };
            reg.definition(d)?;
            reg.request_owner_participant::<Adapter<1>>(a.clone())?;
            if i > 0 && self.missing != Some(i) {
                reg.request_owner_participant::<Adapter<0>>(a.clone())?;
            }
            reg.action_provider(a, Provider)?;
        }
        reg.charter_pending_capacity(
            self.protocol.participants.clone(),
            self.protocol.actions.clone(),
            self.protocol.registry_revocation.clone(),
        )?;
        Ok(())
    }
}
fn engine(reverse: bool, missing: Option<usize>, bad_family: bool) -> TestResult<Engine> {
    engine_with(reverse, missing, bad_family, Index::Exact)
}
fn engine_with(
    reverse: bool,
    missing: Option<usize>,
    bad_family: bool,
    index: Index,
) -> TestResult<Engine> {
    let f = fixture(true)?;
    let protocol = f.store.registry.charter_pending.clone().ok_or("protocol")?;
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(Module::<0> {
            protocol: protocol.clone(),
            missing,
            bad_family,
            index,
        }),
        Box::new(Module::<1> {
            protocol,
            missing,
            bad_family,
            index,
        }),
    ];
    if reverse {
        modules.reverse();
    }
    Ok(Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        modules,
    )?)
}
#[test]
fn pending_real_engine_requires_each_exact_owner_grant_and_family_bound() -> TestResult {
    for reverse in [false, true] {
        engine(reverse, None, false)?;
        for i in 1..5 {
            assert!(
                engine(reverse, Some(i), false).is_err(),
                "missing exact owner grant accepted"
            );
        }
        assert!(
            engine(reverse, None, true).is_err(),
            "wrong complete family ceiling accepted"
        );
    }
    Ok(())
}

/// 17g C2f (decision 4, fail fast at composition): the pending-charter
/// composition requires its owner-index family registered with exactly its
/// owner's descriptor; a composition without it, or with another bound,
/// refuses at boot with the owner-index reason, never at a later Offer.
#[test]
fn pending_composition_requires_the_exact_buyer_offer_index_family() -> TestResult {
    for reverse in [false, true] {
        engine_with(reverse, None, false, Index::Exact)?;
        for (index, reason) in [
            (Index::Missing, "owner index family unavailable"),
            (Index::Descriptor, "owner index descriptor differs"),
        ] {
            let refused = engine_with(reverse, None, false, index).err();
            assert!(
                refused
                    .as_ref()
                    .is_some_and(|e| e.to_string().contains(reason)),
                "{index:?}: {refused:?}"
            );
        }
    }
    Ok(())
}
