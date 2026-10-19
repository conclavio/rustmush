//! Registration capacity and same actual state-slot coexistence. No business
//! authority or image allowance is supplied by these registration fixtures.
use super::*;
use crate::action::{ActionError, ActionProvider, ActionProviderRegistryBuilder};
use crate::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder,
};
use crate::{Registrar, Registry};
use serde_json::json;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Default)]
struct Cell<const SLOT: u8>;
struct Adapter<const ID: u8, const SLOT: u8>;
struct Bound;
impl BoundParticipant for Bound {
    fn apply(self) {}
}
impl<const ID: u8, const SLOT: u8> OwnerParticipant for Adapter<ID, SLOT> {
    type State = Cell<SLOT>;
    type Request = ();
    type Output = ();
    type Patch = ();
    type Bound<'a> = Bound;
    fn prepare(
        &self,
        _: &Self::State,
        _: &OwnerPrepareContext<'_>,
        _: (),
    ) -> Result<OwnerPreparation<(), ()>, ParticipantError> {
        Err(ParticipantError::Unavailable)
    }
    fn recheck(
        &self,
        _: &Self::State,
        _: &OwnerPrepareContext<'_>,
        _: &(),
    ) -> Result<(), ParticipantError> {
        Err(ParticipantError::Unavailable)
    }
    fn bind<'a>(self: Arc<Self>, _: &'a mut Self::State, _: ()) -> Result<Bound, ParticipantError> {
        Err(ParticipantError::Unavailable)
    }
    fn accepts_write(&self, _: &ReplayWrite) -> bool {
        false
    }
}
struct Provider;
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}
fn reference(owner: &OwnerId, kind: &str, id: &str) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        owner.clone(),
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            1,
            json!({}),
        )?,
    )?)
}
fn registration<const ID: u8, const SLOT: u8>(
    builder: &mut ParticipantRegistryBuilder,
    owner: &OwnerId,
    action: &DefinitionRef,
) -> TestResult {
    builder.register(
        owner.clone(),
        reference(owner, "owner_participant", &format!("slot_{ID}"))?,
        BTreeSet::from([action.clone()]),
        Adapter::<ID, SLOT>,
    )?;
    Ok(())
}
#[test]
fn sixteen_distinct_registered_slots_fit_and_seventeenth_refuses_without_changing_invocation_caps()
-> TestResult {
    let owner = OwnerId::new("fixture")?;
    let action = reference(&owner, "action", "write")?;
    let mut builder = ParticipantRegistryBuilder::default();
    registration::<0, 0>(&mut builder, &owner, &action)?;
    registration::<1, 1>(&mut builder, &owner, &action)?;
    registration::<2, 2>(&mut builder, &owner, &action)?;
    registration::<3, 3>(&mut builder, &owner, &action)?;
    registration::<4, 4>(&mut builder, &owner, &action)?;
    registration::<5, 5>(&mut builder, &owner, &action)?;
    registration::<6, 6>(&mut builder, &owner, &action)?;
    registration::<7, 7>(&mut builder, &owner, &action)?;
    registration::<8, 8>(&mut builder, &owner, &action)?;
    registration::<9, 9>(&mut builder, &owner, &action)?;
    registration::<10, 10>(&mut builder, &owner, &action)?;
    registration::<11, 11>(&mut builder, &owner, &action)?;
    registration::<12, 12>(&mut builder, &owner, &action)?;
    registration::<13, 13>(&mut builder, &owner, &action)?;
    registration::<14, 14>(&mut builder, &owner, &action)?;
    registration::<15, 15>(&mut builder, &owner, &action)?;
    assert_eq!(builder.registrations.len(), 16);
    assert!(matches!(
        builder.register(
            owner.clone(),
            reference(&owner, "owner_participant", "slot_16")?,
            BTreeSet::from([action]),
            Adapter::<16, 16>
        ),
        Err(ParticipantError::RegistrationLimit)
    ));
    assert_eq!(MAX_OWNER_PARTICIPANT_IMAGES, 3);
    assert_eq!(MAX_OWNER_PARTICIPANT_MODULES, 2);
    Ok(())
}
#[test]
fn same_owner_same_slot_disjoint_actions_freeze_but_ambiguous_registrations_refuse() -> TestResult {
    let owner = OwnerId::new("fixture")?;
    let mut definitions = DefinitionRegistryBuilder::new();
    let mut define = |kind: &str, id: &str| -> TestResult<DefinitionRef> {
        Ok(definitions.register(
            owner.clone(),
            DefinitionDescriptor::new(
                DefinitionKind::new(kind)?,
                DefinitionId::new(id)?,
                1,
                json!({}),
            )?,
        )?)
    };
    let first = define("action", "first")?;
    let second = define("action", "second")?;
    let one = define("owner_participant", "one")?;
    let two = define("owner_participant", "two")?;
    let owners = BTreeSet::from([owner.clone()]);
    let ready = MechanicalOwnerReadiness::for_owners(&owners);
    let mut definitions = definitions.freeze()?;
    definitions.bind_owner_readiness(ready.clone());
    let mut providers = ActionProviderRegistryBuilder::new();
    providers.register(owner.clone(), first.clone(), Arc::new(Provider))?;
    providers.register(owner.clone(), second.clone(), Arc::new(Provider))?;
    let providers =
        providers.freeze_for_owners_with_readiness(&definitions, &owners, ready.clone())?;
    let mut states = Registry::new();
    Registrar::new(&mut states, "fixture").state(Cell::<0>);
    let slots = states.take_state_inits();
    let mut builder = ParticipantRegistryBuilder::default();
    builder.register(
        owner.clone(),
        one.clone(),
        BTreeSet::from([first.clone()]),
        Adapter::<0, 0>,
    )?;
    builder.register(
        owner.clone(),
        two.clone(),
        BTreeSet::from([second.clone()]),
        Adapter::<1, 0>,
    )?;
    builder.request::<Adapter<0, 0>>(owner.clone(), first.clone())?;
    builder.request::<Adapter<1, 0>>(owner.clone(), second.clone())?;
    let frozen = builder.freeze(&definitions, &providers, &slots, ready)?;
    assert!(frozen.grants_exact(&one, &owner, &first));
    assert!(frozen.grants_exact(&two, &owner, &second));
    for mode in 0..4 {
        let mut builder = ParticipantRegistryBuilder::default();
        builder.register(
            owner.clone(),
            one.clone(),
            BTreeSet::from([first.clone()]),
            Adapter::<0, 0>,
        )?;
        let result = match mode {
            0 => builder.register(
                owner.clone(),
                two.clone(),
                BTreeSet::from([first.clone()]),
                Adapter::<1, 0>,
            ),
            1 => {
                let foreign = OwnerId::new("foreign")?;
                builder.register(
                    foreign.clone(),
                    reference(&foreign, "owner_participant", "foreign")?,
                    BTreeSet::from([second.clone()]),
                    Adapter::<1, 0>,
                )
            }
            2 => builder.register(
                owner.clone(),
                two.clone(),
                BTreeSet::from([second.clone()]),
                Adapter::<0, 0>,
            ),
            _ => builder.register(
                owner.clone(),
                one.clone(),
                BTreeSet::from([second.clone()]),
                Adapter::<1, 1>,
            ),
        };
        assert!(matches!(result, Err(ParticipantError::Registration)));
    }
    Ok(())
}
