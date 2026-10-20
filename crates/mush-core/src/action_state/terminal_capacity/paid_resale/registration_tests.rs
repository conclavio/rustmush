//! Actual Engine registration. Inert adapters always refuse preparation and
//! cannot mint native or Roll business authority from these fixture rows.
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
    fn bind<'a>(self: Arc<Self>, _: &'a mut Cell<N>, _: ()) -> Result<Never, ParticipantError> {
        Err(ParticipantError::Authority)
    }
    fn accepts_write(&self, _: &crate::ReplayWrite) -> bool {
        false
    }
}
fn me(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(me)?,
        DefinitionId::new(id).map_err(me)?,
        1,
        payload,
    )
    .map_err(me)
}
fn participant(n: usize) -> Result<DefinitionDescriptor, ModError> {
    descriptor(
        "owner_participant",
        if n == 0 {
            "initial_filing"
        } else {
            "commercial_roots"
        },
        if n == 0 {
            json!({"generation":0})
        } else {
            json!({"format":1,"protocol":"commercial_retention_v1"})
        },
    )
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
                Err("exact resale fixture reference".into())
            }
        },
    )
}
#[derive(Clone)]
struct Config {
    manifest: DefinitionRef,
    participants: [DefinitionRef; 2],
    purchase: [DefinitionRef; 3],
    resale: [DefinitionRef; 5],
    registry: DefinitionRef,
    missing: Option<(usize, usize)>,
    bad_family: bool,
}
struct Module<const N: usize>(Config);
impl<const N: usize> GameModule for Module<N> {
    fn name(&self) -> &'static str {
        match N {
            0 => "economy",
            1 => "roll",
            2 => "market",
            _ => "tacsim",
        }
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let c = &self.0;
        if N == 3 {
            checked(reg, "registry_manifest", vec![c.manifest.clone()])?;
            reg.definition(descriptor(
                "registry_manifest",
                "combat",
                json!({"generation":0}),
            )?)?;
            return Ok(());
        }
        let all = c
            .purchase
            .iter()
            .chain(c.resale.iter())
            .chain([&c.registry])
            .cloned()
            .collect::<Vec<_>>();
        if N < 2 {
            checked(reg, "owner_participant", vec![c.participants[N].clone()])?;
            let reference = reg.definition(participant(N)?)?;
            if N == 0 {
                reg.state(Cell::<0>);
                reg.owner_participant(
                    reference,
                    all.into_iter().filter(|a| a != &c.resale[0]).collect(),
                    Adapter::<0>,
                )?;
                return Ok(());
            }
            reg.state(Cell::<1>);
            reg.owner_participant(
                reference,
                all.into_iter().filter(|a| a != &c.resale[0]).collect(),
                Adapter::<1>,
            )?;
        }
        let families = super::super::FAMILIES.into_iter().chain([
            ("market", super::super::QUOTE, 16384),
            ("roll", super::super::DEPLOYMENT, 4096),
            ("market", OFFER, 16384),
            ("roll", ROLL, 40960),
            ("market", MARKET, 40960),
            ("roll", HEAD, 4096),
            ("roll", HOLDER, 32768),
            ("market", TERMINAL, 8192),
            ("roll", SANCTION, 32768),
            ("roll", STATUS, 4096),
        ]);
        for (owner, family, max) in
            families.filter(|(owner, _, _)| *owner == if N == 1 { "roll" } else { "market" })
        {
            let schema =
                |suffix: &str| CanonicalId::new(format!("{family}.{suffix}.v1")).map_err(me);
            let (decode, validate, key) = if owner == "roll" {
                ("strict_decode", "invariants", 192)
            } else {
                ("decode", "validate", 32)
            };
            let d = StateFamilyDescriptor::new(
                StateFamilyId::new(family).map_err(me)?,
                schema("key")?,
                schema("value")?,
                1,
                schema(decode)?,
                schema(validate)?,
                schema("canonical")?,
            )
            .map_err(me)?
            .with_limits(
                key,
                if c.bad_family && family == ROLL {
                    max - 1
                } else {
                    max
                },
            )
            .map_err(me)?;
            let h = reg.state_family(
                d,
                |k: &String| Ok(k.clone()),
                |v: &Value| Ok(v.clone()),
                |_: &Value| Ok(()),
                |v: &Value| Ok(v.clone()),
            )?;
            if owner == "roll" && family != SANCTION && family != STATUS {
                reg.delegate_state_family(&h, "market")?;
            }
            if owner == "market" && [MARKET, TERMINAL].contains(&family) {
                reg.delegate_state_family(&h, "roll")?;
            }
        }
        if N == 1 {
            let mut defs = DefinitionRegistryBuilder::new();
            let authority_ref = defs
                .register(
                    OwnerId::new("roll").map_err(me)?,
                    descriptor("registry_authority", "mind_revocation", authority())?,
                )
                .map_err(me)?;
            checked(reg, "registry_authority", vec![authority_ref.clone()])?;
            reg.definition(descriptor(
                "registry_authority",
                "mind_revocation",
                authority(),
            )?)?;
            checked(reg, "action", vec![c.registry.clone()])?;
            reg.definition(descriptor("action","registry_term_revocation",json!({"format":1,"protocol":"registry_exact_term_revocation_v1","reason":"registry_review","scope":"one_accepted_term_generation","duration":"original_grace_end","later_terms":"fresh_admission","shared_rating":"unchanged"}))?.with_references(vec![authority_ref,c.participants[1].clone()]))?;
            if c.missing != Some((5, 1)) {
                reg.request_owner_participant::<Adapter<1>>(c.registry.clone())?;
            }
            if c.missing != Some((5, 0)) {
                reg.request_owner_participant::<Adapter<0>>(c.registry.clone())?;
            }
            reg.action_provider(c.registry.clone(), Provider)?;
        } else {
            checked(
                reg,
                "action",
                c.purchase.iter().chain(c.resale.iter()).cloned().collect(),
            )?;
            for (i, (id, transition)) in super::super::ACTIONS
                .into_iter()
                .zip(super::super::TRANSITIONS)
                .enumerate()
            {
                reg.definition(
                    descriptor(
                        "action",
                        id,
                        super::super::action_payload(1, transition).map_err(me)?,
                    )?
                    .with_references(vec![
                        c.manifest.clone(),
                        c.participants[0].clone(),
                        c.participants[1].clone(),
                    ]),
                )?;
                reg.request_owner_participant::<Adapter<0>>(c.purchase[i].clone())?;
                reg.request_owner_participant::<Adapter<1>>(c.purchase[i].clone())?;
                reg.action_provider(c.purchase[i].clone(), Provider)?;
            }
            for (i, (id, payload)) in action_payloads().into_iter().enumerate() {
                reg.definition(
                    descriptor("action", id, payload)?
                        .with_references(vec![c.participants[1].clone()]),
                )?;
                // Match the real Market-only Offer: no owner grant or request.
                if i > 0 && c.missing != Some((i, 1)) {
                    reg.request_owner_participant::<Adapter<1>>(c.resale[i].clone())?;
                }
                if i > 0 && c.missing != Some((i, 0)) {
                    reg.request_owner_participant::<Adapter<0>>(c.resale[i].clone())?;
                }
                reg.action_provider(c.resale[i].clone(), Provider)?;
            }
            reg.original_mind_purchase_capacity(
                c.manifest.clone(),
                c.participants.clone(),
                c.purchase.clone(),
            )?;
            reg.paid_resale_capacity(
                c.participants.clone(),
                c.resale.clone(),
                Some(c.registry.clone()),
            )?;
        }
        Ok(())
    }
}
fn engine(
    reverse: bool,
    missing: Option<(usize, usize)>,
    bad_family: bool,
) -> Result<Engine, Box<dyn std::error::Error>> {
    let f = fixture_product_resale_registry(0, 0, 1, "sealed", true, true)?;
    let old = f
        .store
        .registry
        .original_purchase
        .as_ref()
        .ok_or("old profile")?;
    let c = Config {
        manifest: old.manifest.clone(),
        participants: old.participants.clone(),
        purchase: old.actions.clone(),
        resale: actions(&f),
        registry: f.resale_registry.clone().ok_or("Registry")?,
        missing,
        bad_family,
    };
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(Module::<0>(c.clone())),
        Box::new(Module::<1>(c.clone())),
        Box::new(Module::<2>(c.clone())),
        Box::new(Module::<3>(c)),
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
fn actual_composition_requires_every_owner_grant_and_exact_future_family_bound() -> TestResult {
    for reverse in [false, true] {
        // Offer has no Economy or Roll participant grant; every action
        // that writes those owner images still requires both exact requests.
        engine(reverse, None, false)?;
        for action in 1..6 {
            for owner in 0..2 {
                assert!(
                    engine(reverse, Some((action, owner)), false).is_err(),
                    "missing actual owner request accepted"
                );
            }
        }
        assert!(
            engine(reverse, None, true).is_err(),
            "different complete family limit accepted"
        );
    }
    Ok(())
}
