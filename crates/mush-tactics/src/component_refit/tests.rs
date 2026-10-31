#![cfg(test)]

//! Actual Engine registration, preparation and installation of the refit
//! participant (doc 31 cut UD.4b). The test policy checks hull ownership only;
//! this is not a facility, fee, title, work-order or part-custody fixture.
use super::*;
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, AuthoritativeActionOutput, CanonicalHash, IdempotencyKey, IdempotencyScope,
    PerceivedRef, PreparedAuthoritativeAction, PreparedCausalRef, PrincipalId, RequestUid,
    ScopedIdempotencyKey,
};
use mush_core::action_state::{StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionTxn, CanonicalId, DefinitionRegistryBuilder, DefinitionValidationContext, GOD,
    GameModule, ObjKind, PrepareCtx, RevisionSet, RulesValue,
};
use mush_testkit::TestWorld;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// What the test policy fits: the second unit, `gun_a` out and `gun_b` in —
/// or, with `wrong_mount`, the third unit, whose `gun_b` sits elsewhere.
#[derive(Clone)]
struct Policy {
    specs: Vec<crate::UnitSpec>,
}
struct PolicyRequest {
    target: usize,
}
impl RefitPolicy for Policy {
    type Request = PolicyRequest;
    fn authorize(
        &self,
        c: &OwnerPrepareContext<'_>,
        r: &PolicyRequest,
        current: &TacticalUnit,
    ) -> std::result::Result<RefitChoice, ParticipantError> {
        let hull = c.world().get(current.object()).ok_or_else(refused)?;
        if hull.owner != c.invocation().actor() {
            return Err(refused());
        }
        let spec = self.specs.get(r.target).ok_or_else(refused)?;
        let baseline = TacticalUnit::from_spec(spec, current.object()).map_err(|_| refused())?;
        let gun_b = ComponentId::new("gun_b").map_err(|_| refused())?;
        let fresh = ComponentState::capture(&baseline, &gun_b).map_err(|_| refused())?;
        Ok(RefitChoice::new(
            spec.definition.clone(),
            Some(ComponentId::new("gun_a").map_err(|_| refused())?),
            Some((gun_b, fresh)),
        ))
    }
}

// The fixture provider's own typed half: a composed action must write one.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureReceipt {
    format: u8,
    unit: ObjId,
    from: u64,
    to: u64,
}
struct Provider {
    receipts: StateFamilyHandle<CommitToken, FixtureReceipt>,
    handle: ParticipantHandle<ComponentRefitParticipant<Policy>>,
    outcome: DefinitionRef,
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> std::result::Result<AuthoritativeActionOutput, ActionError> {
        let error = || {
            ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "refit fixture refused",
            )
            .expect("static error")
        };
        let object =
            ObjId(serde_json::from_value(inv.parameters()["hull"].clone()).map_err(|_| error())?);
        let revision =
            serde_json::from_value(inv.parameters()["revision"].clone()).map_err(|_| error())?;
        let target =
            serde_json::from_value(inv.parameters()["target"].clone()).map_err(|_| error())?;
        let output = ctx
            .prepare_owner(
                &self.handle,
                RefitRequest::new(object, revision, PolicyRequest { target }),
            )
            .map_err(|_| error())?;
        txn.insert(
            &self.receipts,
            &output.commit(),
            FixtureReceipt {
                format: 1,
                unit: output.journal().object,
                from: output.journal().from_revision,
                to: output.journal().to_revision,
            },
        )
        .map_err(|_| error())?;
        let result = RulesValue::new(
            self.outcome.clone(),
            json!({"from": output.journal().from_revision, "to": output.journal().to_revision}),
        )
        .map_err(|_| error())?;
        Ok(AuthoritativeActionOutput::new(result).with_definitions(vec![self.outcome.clone()]))
    }
}

#[derive(Clone)]
struct Rules {
    descriptors: Vec<DefinitionDescriptor>,
    specs: Vec<crate::UnitSpec>,
    action: DefinitionRef,
    outcome: DefinitionRef,
}
fn descriptor(kind: &str, id: &str, value: Value) -> Result<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new(kind)?,
        DefinitionId::new(id)?,
        1,
        value,
    )?)
}
/// Three units: `unit_1` (radio, `gun_a` on the hull), `unit_2` (the same
/// radio under its own unit-scoped definition, `gun_b` on the hull) and
/// `unit_3` (`gun_b` on the turret: a different mount).
fn rules() -> Result<Rules> {
    let owner = OwnerId::new("bay_fixture")?;
    let mut builder = DefinitionRegistryBuilder::new();
    let mut descriptors = vec![];
    let schema = descriptor("schema", "rules", json!({}))?;
    let schema_ref = builder.register(owner.clone(), schema.clone())?;
    descriptors.push(schema);
    let mut specs = vec![];
    for (n, gun, mount) in [
        (1, "gun_a", "hull"),
        (2, "gun_b", "hull"),
        (3, "gun_b", "turret"),
    ] {
        let radio = descriptor(
            "component",
            &format!("unit_{n}.radio"),
            json!({"role": "radio"}),
        )?;
        let radio_ref = builder.register(owner.clone(), radio.clone())?;
        descriptors.push(radio);
        let weapon = descriptor(
            "component",
            &format!("unit_{n}.{gun}"),
            json!({"role": gun}),
        )?;
        let weapon_ref = builder.register(owner.clone(), weapon.clone())?;
        descriptors.push(weapon);
        let section = |id: &str| -> Result<crate::SectionSpec> {
            Ok(crate::SectionSpec {
                id: crate::SectionId::new(id)?,
                condition: crate::ConditionId::new("ok")?,
                facets: vec![],
                slots: vec![],
                cells: vec![],
            })
        };
        let body = crate::UnitSpecBody {
            sections: vec![section("hull")?, section("turret")?],
            unit_cells: vec![crate::CellSpec {
                id: crate::CellId::new("damage")?,
                min: 0,
                max: 30,
                init: 0,
            }],
            components: vec![
                crate::ComponentSpec {
                    id: ComponentId::new("radio")?,
                    definition: radio_ref,
                    install: crate::InstallPoint::Section(crate::SectionId::new("hull")?),
                    condition: crate::ConditionId::new("ok")?,
                    cells: vec![],
                    asset: None,
                },
                crate::ComponentSpec {
                    id: ComponentId::new(gun)?,
                    definition: weapon_ref,
                    install: crate::InstallPoint::Section(crate::SectionId::new(mount)?),
                    condition: crate::ConditionId::new("ok")?,
                    cells: vec![],
                    asset: None,
                },
            ],
            capabilities: vec![],
            mobility: None,
            rules: RulesValue::new(schema_ref.clone(), json!({}))?,
        };
        let d = body
            .descriptor(
                DefinitionKind::new("unit")?,
                DefinitionId::new(format!("unit_{n}"))?,
                1,
            )?
            .as_current();
        let reference = builder.register(owner.clone(), d.clone())?;
        descriptors.push(d);
        specs.push(body.with_definition(reference));
    }
    let out = descriptor("outcome", "refitted", json!({}))?;
    let outcome = builder.register(owner.clone(), out.clone())?;
    descriptors.push(out);
    let act = descriptor("action", "bay_refit", json!({}))?;
    let action = builder.register(owner, act.clone())?;
    descriptors.push(act);
    Ok(Rules {
        descriptors,
        specs,
        action,
        outcome,
    })
}
impl GameModule for Rules {
    fn name(&self) -> &'static str {
        "bay_fixture"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let mut kinds: BTreeMap<DefinitionKind, Vec<DefinitionDescriptor>> = BTreeMap::new();
        for d in &self.descriptors {
            kinds.entry(d.kind().clone()).or_default().push(d.clone());
        }
        for (kind, expected) in kinds {
            reg.definition_validator(kind, move |ctx: &DefinitionValidationContext<'_>| {
                let e = ctx.definition();
                if expected.iter().any(|d| {
                    d.id() == &e.reference().key().id
                        && d.payload() == e.payload()
                        && d.references() == e.references()
                        && d.is_current() == e.is_current()
                }) {
                    Ok(())
                } else {
                    Err("test definition drift".into())
                }
            })?;
        }
        for d in &self.descriptors {
            reg.definition(d.clone())?;
        }
        let id = |s: &str| CanonicalId::new(s).map_err(|e| ModError::msg(e.to_string()));
        let receipts = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("refit_fixture_receipt")
                    .map_err(|e| ModError::msg(e.to_string()))?,
                id("refit_fixture_receipt.key.v1")?,
                id("refit_fixture_receipt.value.v1")?,
                1,
                id("refit_fixture_receipt.decode.v1")?,
                id("refit_fixture_receipt.validate.v1")?,
                id("refit_fixture_receipt.canonical.v1")?,
            )
            .map_err(|e| ModError::msg(e.to_string()))?
            .with_limits(32, 4096)
            .map_err(|e| ModError::msg(e.to_string()))?,
            |key: &CommitToken| Ok(key.to_hex()),
            |value: &Value| {
                serde_json::from_value::<FixtureReceipt>(value.clone())
                    .map_err(|_| "fixture receipt encoding".into())
            },
            |row: &FixtureReceipt| {
                if row.format == 1 && row.unit.0 != 0 && row.from.checked_add(1) == Some(row.to) {
                    Ok(())
                } else {
                    Err("fixture receipt shape".into())
                }
            },
            |row: &FixtureReceipt| {
                serde_json::to_value(row).map_err(|_| "fixture receipt encoding".into())
            },
        )?;
        let handle = reg
            .request_owner_participant::<ComponentRefitParticipant<Policy>>(self.action.clone())?;
        reg.action_provider(
            self.action.clone(),
            Provider {
                receipts,
                handle,
                outcome: self.outcome.clone(),
            },
        )?;
        Ok(())
    }
}
fn modules(r: &Rules) -> Result<Vec<Box<dyn GameModule>>> {
    let mut catalog = crate::UnitCatalogBuilder::new();
    let vocabulary = crate::MobilityVocabulary::base().freeze();
    for spec in &r.specs {
        catalog.add(spec.clone(), &vocabulary)?;
    }
    let tactics = crate::TacticsModule::with_catalog(catalog.freeze()).with_component_refit(
        Policy {
            specs: r.specs.clone(),
        },
        r.action.clone(),
        r.specs.iter().map(|s| s.definition.clone()).collect(),
        Vec::new(),
    );
    Ok(vec![
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(r.clone()),
        Box::new(tactics),
    ])
}
struct Fixture {
    world: TestWorld,
    rules: Rules,
    hull: ObjId,
}
fn fixture() -> Result<Fixture> {
    let rules = rules()?;
    let mut world = TestWorld::with_modules(modules(&rules)?);
    let hull = world
        .world_mut()
        .create(ObjKind::Thing, "Bay hull", GOD, None)?;
    world.run(GOD, &format!("@unit/create #{}=unit_1", hull.0));
    let unit = stored_unit(&world, hull).ok_or("actual unit creation")?;
    let plan = crate::UnitMutationPlan::new(
        vec![crate::UnitMutationOp::SetCell {
            addr: crate::CellAddr::Unit {
                cell: crate::CellId::new("damage")?,
            },
            value: 7,
        }],
        Some(unit.revision()),
        Some("retained damage".into()),
    )?;
    world.run(
        GOD,
        &format!("@unit/damage #{}={}", hull.0, serde_json::to_string(&plan)?),
    );
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    Ok(Fixture { world, rules, hull })
}
fn stored_unit(world: &TestWorld, hull: ObjId) -> Option<TacticalUnit> {
    world
        .world()
        .module_state::<UnitStore>()
        .and_then(|s| s.units.get(&hull))
        .cloned()
}
fn token() -> CommitToken {
    CommitToken::from_bytes([7; 16])
}
fn invocation(f: &Fixture, target: usize) -> Result<ActionInvocation> {
    let request = RequestUid::from_bytes([8; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let revision = stored_unit(&f.world, f.hull).ok_or("unit")?.revision();
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: GOD,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"hull": f.hull.0, "revision": revision, "target": target}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("bay_fixture")?,
                namespace: CanonicalId::new("bay_refit")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
fn bundle(
    p: &mush_core::composed::PreparedComposedTxn,
    o: PreparedAuthoritativeAction,
    inv: &ActionInvocation,
    action: DefinitionRef,
) -> Result<AuthoritativeBundle> {
    let (outcome, resolution, mut definitions, mut witnesses, mut outbox, events) = o.into_parts();
    definitions.push(action.clone());
    definitions.push(outcome.schema().clone());
    definitions.sort();
    definitions.dedup();
    if witnesses.is_empty() {
        witnesses.push(inv.principal().clone());
    }
    witnesses.sort();
    witnesses.dedup();
    outbox.sort_by_key(|x| x.ordinal());
    Ok(AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([3; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([4; 32]),
        action,
        provider: p.owner().clone(),
        cause: PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: p.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token())),
        semantic_events: events,
        eligible_witnesses: witnesses,
        durable_outbox: outbox,
    })?)
}

/// **A composed refit moves the hull mount by mount** (doc 31 cut UD.4b): to
/// the second unit, `gun_a` out and `gun_b` in, the radio's definition
/// retargeted to the target's own (the same component under a unit-scoped id),
/// the hull's damage kept, one exact Tactics write.
#[test]
fn ud4b_a_composed_refit_swaps_by_mount_and_keeps_the_hulls_damage() -> Result {
    let mut f = fixture()?;
    let before = stored_unit(&f.world, f.hull).ok_or("before")?;
    let inv = invocation(&f, 1)?;
    let (p, o) = f
        .world
        .engine_mut()
        .prepare_registered_composed_authoritative_action(&f.rules.action, &inv, token())?;
    let writes = p.authoritative_replay_writes();
    check_eq!(
        writes.len(),
        2,
        "the Tactics unit image and the provider's receipt"
    );
    check_eq!(stored_unit(&f.world, f.hull).as_ref(), Some(&before));
    let b = bundle(&p, o, &inv, f.rules.action.clone())?;
    let g = f
        .world
        .registry()
        .server_guard_set_builder(RevisionSet::default())?
        .finish()?;
    f.world.engine_mut().stage_authoritative_action(p, g, b)?;
    let held = f.world.engine_mut().take_staged_action()?.ok_or("staged")?;
    let (_, installation) = held.into_parts();
    f.world.engine_mut().install_composed_action(installation)?;
    let after = stored_unit(&f.world, f.hull).ok_or("after")?;
    let target = f.rules.specs.get(1).ok_or("target")?;
    check_eq!(after.schema(), &target.definition);
    check_eq!(after.revision(), before.revision() + 1);
    check!(after.component(&ComponentId::new("gun_a")?).is_none());
    check!(after.component(&ComponentId::new("gun_b")?).is_some());
    let radio = ComponentId::new("radio")?;
    let target_radio = target
        .components
        .iter()
        .find(|c| c.id == radio)
        .ok_or("radio")?;
    check_eq!(
        after.component(&radio).map(|c| &c.definition),
        Some(&target_radio.definition)
    );
    check_eq!(
        after
            .cell(&crate::CellAddr::Unit {
                cell: crate::CellId::new("damage")?
            })
            .map(|c| c.current),
        Some(7)
    );
    Ok(())
}

/// A refit whose part goes onto another mount is refused at preparation and
/// changes nothing.
#[test]
fn ud4b_a_refit_onto_another_mount_is_refused() -> Result {
    let mut f = fixture()?;
    let before = stored_unit(&f.world, f.hull).ok_or("before")?;
    let inv = invocation(&f, 2)?;
    let prepared = f
        .world
        .engine_mut()
        .prepare_registered_composed_authoritative_action(&f.rules.action, &inv, token());
    check!(prepared.is_err());
    check_eq!(stored_unit(&f.world, f.hull).as_ref(), Some(&before));
    Ok(())
}
