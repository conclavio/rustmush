#![cfg(test)]

//! Actual Engine registration/preparation/installation tests. The test policy
//! checks live core object ownership only; this is not a commercial title,
//! certified core, fee, facility, work-order or custody publication fixture.
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
    GameModule, PrepareCtx, RevisionSet, RulesValue,
};
use mush_testkit::TestWorld;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone)]
struct Policy {
    specs: Vec<crate::UnitSpec>,
}
struct PolicyRequest {
    target: usize,
    asset: ObjId,
    expected_actor: ObjId,
    expected_commit: CommitToken,
}
impl ReplacementPolicy for Policy {
    type Request = PolicyRequest;
    fn authorize(
        &self,
        c: &OwnerPrepareContext<'_>,
        r: &PolicyRequest,
        current: &TacticalUnit,
    ) -> std::result::Result<ReplacementChoice, ParticipantError> {
        if c.invocation().actor() != r.expected_actor || c.commit_token() != r.expected_commit {
            return Err(refused());
        }
        let hull = c.world().get(current.object()).ok_or_else(refused)?;
        let asset = c.world().get(r.asset).ok_or_else(refused)?;
        if hull.owner != c.invocation().actor()
            || asset.owner != hull.owner
            || asset.kind != ObjKind::Thing
            || asset.flags.contains(Flags::GOING)
            || asset.location != hull.location
        {
            return Err(refused());
        }
        let spec = self.specs.get(r.target).ok_or_else(refused)?;
        let mut donor = TacticalUnit::from_spec(spec, ObjId(500)).map_err(|_| refused())?;
        let core = ComponentId::new("core").map_err(|_| refused())?;
        let plan = crate::UnitMutationPlan::new(
            vec![crate::UnitMutationOp::SetComponentAsset {
                component: core.clone(),
                asset: Some(r.asset),
            }],
            Some(donor.revision()),
            Some("test incoming object".into()),
        )
        .map_err(|_| refused())?;
        donor = crate::apply(&donor, &plan, c.world().now())
            .map_err(|_| refused())?
            .0;
        Ok(ReplacementChoice::new(
            spec.definition.clone(),
            ComponentState::capture(&donor, &core).map_err(|_| refused())?,
        ))
    }
}
// The fixture producer records its actual owner result in the typed half of
// the composed action. This is an adapter-test receipt, not a commercial fee,
// certified core, title or custody proof.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureReceipt {
    format: u8,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    unit: ObjId,
    from: u64,
    to: u64,
}
struct Provider {
    receipts: StateFamilyHandle<CommitToken, FixtureReceipt>,
    handle: ParticipantHandle<ComponentReplacementParticipant<Policy>>,
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
        let prepare = || -> Result<_> {
            let object = ObjId(serde_json::from_value(inv.parameters()["hull"].clone())?);
            let asset = ObjId(serde_json::from_value(inv.parameters()["asset"].clone())?);
            let expected_actor = ObjId(serde_json::from_value(inv.parameters()["actor"].clone())?);
            let revision = serde_json::from_value(inv.parameters()["revision"].clone())?;
            let expected_commit = serde_json::from_value(inv.parameters()["commit"].clone())?;
            Ok(ReplacementRequest::new(
                object,
                revision,
                PolicyRequest {
                    target: 1,
                    asset,
                    expected_actor,
                    expected_commit,
                },
            ))
        };
        let error = || {
            ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "replacement fixture refused",
            )
            .expect("static error")
        };
        let request = prepare().map_err(|_| error())?;
        let output = ctx
            .prepare_owner(&self.handle, request)
            .map_err(|_| error())?;
        txn.insert(
            &self.receipts,
            &output.commit(),
            FixtureReceipt {
                format: 1,
                action: output.action().clone(),
                commit: output.commit(),
                actor: output.actor(),
                unit: output.journal().object,
                from: output.journal().from_revision,
                to: output.journal().to_revision,
            },
        )
        .map_err(|_| error())?;
        let result = RulesValue::new(self.outcome.clone(), json!({"action":output.action(),"commit":output.commit(),"actor":output.actor(),"from":output.journal().from_revision,"to":output.journal().to_revision})).map_err(|_|error())?;
        Ok(AuthoritativeActionOutput::new(result).with_definitions(vec![self.outcome.clone()]))
    }
}
#[derive(Clone)]
struct Rules {
    descriptors: Vec<DefinitionDescriptor>,
    specs: Vec<crate::UnitSpec>,
    action: DefinitionRef,
    other: DefinitionRef,
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
fn rules() -> Result<Rules> {
    let owner = OwnerId::new("hardware_fixture")?;
    let mut builder = DefinitionRegistryBuilder::new();
    let mut descriptors = vec![];
    let schema = descriptor("schema", "rules", json!({}))?;
    let schema_ref = builder.register(owner.clone(), schema.clone())?;
    descriptors.push(schema);
    let mut specs = vec![];
    for n in 1..=2 {
        let component = descriptor("component", &format!("core_{n}"), json!({}))?;
        let cref = builder.register(owner.clone(), component.clone())?;
        descriptors.push(component);
        let body = crate::UnitSpecBody {
            sections: vec![crate::SectionSpec {
                id: crate::SectionId::new("hull")?,
                condition: crate::ConditionId::new("ok")?,
                facets: vec![],
                slots: vec![],
                cells: vec![],
            }],
            unit_cells: vec![crate::CellSpec {
                id: crate::CellId::new("heat")?,
                min: 0,
                max: 30,
                init: 0,
            }],
            components: vec![crate::ComponentSpec {
                id: ComponentId::new("core")?,
                definition: cref,
                install: crate::InstallPoint::Unit,
                condition: crate::ConditionId::new("ok")?,
                cells: vec![crate::CellSpec {
                    id: crate::CellId::new("class")?,
                    min: 0,
                    max: n,
                    init: n,
                }],
                asset: None,
            }],
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
    let out = descriptor("outcome", "replaced", json!({}))?;
    let outcome = builder.register(owner.clone(), out.clone())?;
    descriptors.push(out);
    let act = descriptor("action", "replace", json!({}))?;
    let action = builder.register(owner.clone(), act.clone())?;
    descriptors.push(act);
    let act = descriptor("action", "wrong", json!({}))?;
    let other = builder.register(owner, act.clone())?;
    descriptors.push(act);
    Ok(Rules {
        descriptors,
        specs,
        action,
        other,
        outcome,
    })
}
impl GameModule for Rules {
    fn name(&self) -> &'static str {
        "hardware_fixture"
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
        let receipts = reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new("replacement_fixture_receipt")
                    .map_err(|e| ModError::msg(e.to_string()))?,
                CanonicalId::new("replacement_fixture_receipt.key.v1")
                    .map_err(|e| ModError::msg(e.to_string()))?,
                CanonicalId::new("replacement_fixture_receipt.value.v1")
                    .map_err(|e| ModError::msg(e.to_string()))?,
                1,
                CanonicalId::new("replacement_fixture_receipt.decode.v1")
                    .map_err(|e| ModError::msg(e.to_string()))?,
                CanonicalId::new("replacement_fixture_receipt.validate.v1")
                    .map_err(|e| ModError::msg(e.to_string()))?,
                CanonicalId::new("replacement_fixture_receipt.canonical.v1")
                    .map_err(|e| ModError::msg(e.to_string()))?,
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
                if row.format == 1
                    && row.unit.0 != 0
                    && row.actor.0 != 0
                    && row.from.checked_add(1) == Some(row.to)
                {
                    Ok(())
                } else {
                    Err("fixture receipt shape".into())
                }
            },
            |row: &FixtureReceipt| {
                serde_json::to_value(row).map_err(|_| "fixture receipt encoding".into())
            },
        )?;
        let handle = reg.request_owner_participant::<ComponentReplacementParticipant<Policy>>(
            self.action.clone(),
        )?;
        reg.action_provider(
            self.action.clone(),
            Provider {
                receipts: receipts.clone(),
                handle: handle.clone(),
                outcome: self.outcome.clone(),
            },
        )?;
        // The other real provider intentionally retains the first action's
        // opaque handle; core must refuse that crossed registration authority.
        reg.action_provider(
            self.other.clone(),
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
    let tactics = crate::TacticsModule::with_catalog(catalog.freeze()).with_component_replacement(
        Policy {
            specs: r.specs.clone(),
        },
        r.action.clone(),
        ComponentId::new("core")?,
        r.specs.iter().map(|s| s.definition.clone()).collect(),
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
    asset: ObjId,
}
fn fixture() -> Result<Fixture> {
    let rules = rules()?;
    let mut world = TestWorld::with_modules(modules(&rules)?);
    let hull = world
        .world_mut()
        .create(ObjKind::Thing, "Replacement hull", GOD, None)?;
    let asset = world
        .world_mut()
        .create(ObjKind::Thing, "Incoming component", GOD, None)?;
    world.run(GOD, &format!("@unit/create #{}=unit_1", hull.0));
    let unit = world
        .world()
        .module_state::<UnitStore>()
        .and_then(|s| s.units.get(&hull))
        .ok_or("actual unit creation")?;
    let plan = crate::UnitMutationPlan::new(
        vec![crate::UnitMutationOp::SetCell {
            addr: crate::CellAddr::Unit {
                cell: crate::CellId::new("heat")?,
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
    clear(&mut world);
    Ok(Fixture {
        world,
        rules,
        hull,
        asset,
    })
}
fn clear(world: &mut TestWorld) {
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
}
fn invocation(f: &Fixture, actor: ObjId, token: CommitToken) -> Result<ActionInvocation> {
    let request = RequestUid::from_bytes([8; 16]);
    let principal = PrincipalId::Object { id: actor };
    let revision = f
        .world
        .world()
        .module_state::<UnitStore>()
        .and_then(|s| s.units.get(&f.hull))
        .ok_or("unit")?
        .revision();
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"hull":f.hull.0,"asset":f.asset.0,"revision":revision,"actor":GOD.0,"commit":token}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("hardware_fixture")?,
                namespace: CanonicalId::new("replacement")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
fn token() -> CommitToken {
    CommitToken::from_bytes([7; 16])
}
fn prepare(
    f: &mut Fixture,
) -> Result<(
    mush_core::composed::PreparedComposedTxn,
    PreparedAuthoritativeAction,
    ActionInvocation,
)> {
    let inv = invocation(f, GOD, token())?;
    let (p, o) = f
        .world
        .engine_mut()
        .prepare_registered_composed_authoritative_action(&f.rules.action, &inv, token())?;
    Ok((p, o, inv))
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
fn stage(
    f: &mut Fixture,
    p: mush_core::composed::PreparedComposedTxn,
    o: PreparedAuthoritativeAction,
    inv: &ActionInvocation,
) -> Result {
    let b = bundle(&p, o, inv, f.rules.action.clone())?;
    let g = f
        .world
        .registry()
        .server_guard_set_builder(RevisionSet::default())?
        .finish()?;
    f.world.engine_mut().stage_authoritative_action(p, g, b)?;
    Ok(())
}
fn stored(f: &Fixture) -> UnitStore {
    f.world
        .world()
        .module_state::<UnitStore>()
        .expect("actual Tactics slot")
        .clone()
}

#[test]
fn actual_composed_replacement_preserves_damage_and_publishes_matching_lifecycle() -> Result {
    let mut f = fixture()?;
    let before = stored(&f);
    let (p, o, inv) = prepare(&mut f)?;
    let writes = p.authoritative_replay_writes();
    assert_eq!(writes.len(), 2);
    assert!(f.world.engine().action_state().is_empty());
    assert_eq!(stored(&f), before);
    stage(&mut f, p, o, &inv)?;
    let held = f.world.engine_mut().take_staged_action()?.ok_or("staged")?;
    assert_eq!(stored(&f), before);
    let (_, installation) = held.into_parts();
    f.world.engine_mut().install_composed_action(installation)?;
    assert!(!f.world.engine().action_state().is_empty());
    let after = stored(&f);
    let unit = after.units.get(&f.hull).ok_or("after unit")?;
    assert_eq!(
        unit.schema(),
        &f.rules.specs.get(1).ok_or("target")?.definition
    );
    assert_eq!(
        unit.revision(),
        before.units.get(&f.hull).ok_or("before unit")?.revision() + 1
    );
    assert_eq!(
        unit.cell(&crate::CellAddr::Unit {
            cell: crate::CellId::new("heat")?
        })
        .ok_or("heat")?
        .current,
        7
    );
    assert_eq!(
        unit.component(&ComponentId::new("core")?)
            .ok_or("core")?
            .asset,
        Some(f.asset)
    );
    let (_, _, record) = active(
        &after,
        f.hull,
        after.record_definition.as_ref().ok_or("schema")?,
    )?;
    assert_eq!(record.unit.as_ref(), Some(unit));
    let receipt = writes
        .iter()
        .find_map(|write| match write {
            ReplayWrite::ModuleJsonSet {
                namespace, value, ..
            } if namespace.as_str() == "hardware_fixture" => Some(value),
            _ => None,
        })
        .ok_or("actual typed fixture receipt")?;
    let receipt: FixtureReceipt = serde_json::from_value(receipt["value"].clone())?;
    assert_eq!(receipt.action, f.rules.action);
    assert_eq!(receipt.commit, token());
    assert_eq!(receipt.actor, GOD);
    assert_eq!(receipt.unit, f.hull);
    assert_eq!(receipt.from + 1, unit.revision());
    assert_eq!(receipt.to, unit.revision());
    let ReplayWrite::ModuleJsonSet { key, value, .. } = writes
        .iter()
        .find(|write| {
            matches!(write, ReplayWrite::ModuleJsonSet {namespace, ..}
            if namespace.as_str() == "tactics")
        })
        .ok_or("Tactics write")?
    else {
        return Err("wrong write".into());
    };
    assert!(crate::module::replacement_write_is_exact(
        key.as_str(),
        value,
        after.record_definition.as_ref().ok_or("schema")?
    ));
    // Re-run the real Tactics boot decoder against the exact acknowledged image.
    let mut engine = mush_core::Engine::with_modules(
        f.world.world().clone(),
        mush_core::Registry::new(),
        modules(&f.rules)?,
    )?;
    let mut data = std::collections::HashMap::new();
    data.insert(
        "tactics".into(),
        mush_core::ModuleBootData {
            kv: vec![(key.as_str().into(), value.clone())],
            ..Default::default()
        },
    );
    engine.modules_loaded(
        data,
        &mut mush_core::Outbox::new(),
        &mut mush_core::Effects::new(),
    )?;
    assert_eq!(
        engine
            .world
            .module_state::<UnitStore>()
            .and_then(|s| s.units.get(&f.hull)),
        Some(unit)
    );
    Ok(())
}
#[test]
fn stale_lifecycle_and_payload_images_refuse_before_any_install() -> Result {
    for case in 0..6 {
        let mut f = fixture()?;
        let (p, o, inv) = prepare(&mut f)?;
        let s = f
            .world
            .world_mut()
            .module_state_mut::<UnitStore>()
            .ok_or("slot")?;
        let i = s
            .records
            .iter()
            .position(|r| r.object == Some(f.hull))
            .ok_or("record")?;
        match case {
            0 => s.records.get_mut(i).ok_or("row")?.payload_digest = None,
            1 => s.records.get_mut(i).ok_or("row")?.unit = None,
            2 => {
                let row = s.records.get(i).ok_or("row")?.clone();
                s.records.push(row);
            }
            3 => s.record_definition = None,
            4 => s.records.get_mut(i).ok_or("row")?.key_object = Some(ObjId(99)),
            _ => {
                let actual = s.record_definition.as_ref().ok_or("schema")?;
                s.record_definition = Some(DefinitionRef::new(
                    actual.key().clone(),
                    actual.version(),
                    mush_core::DefinitionDigest::from_bytes([91; 32]),
                )?);
            }
        }
        let before = stored(&f);
        assert!(stage(&mut f, p, o, &inv).is_err());
        assert_eq!(stored(&f), before);
    }
    Ok(())
}
#[test]
fn actual_actor_action_commit_and_stale_authority_refuse() -> Result {
    let mut f = fixture()?;
    let before = stored(&f);
    let inv = invocation(&f, GOD, CommitToken::from_bytes([9; 16]))?;
    assert!(
        f.world
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&f.rules.action, &inv, token())
            .is_err()
    );
    let inv = invocation(&f, GOD, token())?;
    assert!(
        f.world
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&f.rules.other, &inv, token())
            .is_err()
    );
    let actor = f
        .world
        .world_mut()
        .create(ObjKind::Player, "Other fitter", GOD, None)?;
    clear(&mut f.world);
    let inv = invocation(&f, actor, token())?;
    assert!(
        f.world
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&f.rules.action, &inv, token())
            .is_err()
    );
    assert_eq!(stored(&f), before);
    let (p, o, inv) = prepare(&mut f)?;
    stage(&mut f, p, o, &inv)?;
    f.world.world_mut().chown(f.asset, actor)?;
    clear(&mut f.world);
    assert!(f.world.engine_mut().take_staged_action().is_err());
    assert_eq!(stored(&f), before);
    Ok(())
}
#[test]
fn actual_deployment_and_unavailable_spatial_ownership_refuse() -> Result {
    let mut f = fixture()?;
    let (p, o, inv) = prepare(&mut f)?;
    stage(&mut f, p, o, &inv)?;
    mush_sim::deployed::mark(
        f.world.world_mut(),
        f.hull,
        mush_sim::DeployedMark::new(42, "real-test-arena"),
    );
    assert!(f.world.engine_mut().take_staged_action().is_err());
    let mut f = fixture()?;
    f.world
        .world_mut()
        .module_state_mut::<mush_sim::JourneyBook>()
        .ok_or("journeys")?
        .held
        .insert("cjourney:bad".into(), "unreadable".into());
    assert!(prepare(&mut f).is_err());
    Ok(())
}
#[test]
fn changed_definition_owner_refuses_at_final_recheck() -> Result {
    let mut f = fixture()?;
    let (p, o, inv) = prepare(&mut f)?;
    stage(&mut f, p, o, &inv)?;
    // The actual shared readiness latch is also held by the adapter's immutable
    // definition snapshot, so a post-prepare owner freeze cannot be cached away.
    let reason = mush_core::RecoveryReason::IntegrityViolation {
        code: CanonicalId::new("replacement_test")?,
    };
    f.world.engine_mut().registry.freeze_mechanics_owner(
        &OwnerId::new("hardware_fixture")?,
        mush_core::IncidentId(1),
        reason,
    )?;
    assert!(f.world.engine_mut().take_staged_action().is_err());
    Ok(())
}

#[test]
fn actual_definition_digest_drift_refuses_frozen_composition() -> Result {
    let mut rules = rules()?;
    let descriptor = rules
        .descriptors
        .iter_mut()
        .find(|d| d.kind().as_str() == "unit" && d.id().as_str() == "unit_2")
        .ok_or("target descriptor")?;
    let mut changed = descriptor.payload().clone();
    *changed
        .get_mut("rules")
        .and_then(|v| v.get_mut("data"))
        .ok_or("target rules")? = json!({"changed":true});
    *descriptor = DefinitionDescriptor::new(
        descriptor.kind().clone(),
        descriptor.id().clone(),
        descriptor.version(),
        changed,
    )?
    .with_references(descriptor.references().to_vec());
    // The real registered root now has a different digest; the retained catalog
    // and adapter still name the original exact root. Final freeze must refuse.
    assert!(
        mush_core::Engine::with_modules(
            mush_core::World::new_minimal(),
            mush_core::Registry::new(),
            modules(&rules)?
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn local_bind_refusal_after_custody_cannot_partially_publish() -> Result {
    let mut f = fixture()?;
    let (prepared, output, inv) = prepare(&mut f)?;
    stage(&mut f, prepared, output, &inv)?;
    let held = f
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("actual custody")?;
    let (_, installation) = held.into_parts();
    let store = f
        .world
        .world_mut()
        .module_state_mut::<UnitStore>()
        .ok_or("slot")?;
    store
        .records
        .iter_mut()
        .find(|r| r.object == Some(f.hull))
        .ok_or("lifecycle")?
        .payload_digest = None;
    let before = stored(&f);
    assert!(
        f.world
            .engine_mut()
            .install_composed_action(installation)
            .is_err()
    );
    assert!(f.world.engine().action_state().is_empty());
    assert_eq!(stored(&f), before);
    Ok(())
}
