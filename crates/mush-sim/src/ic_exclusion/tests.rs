#![cfg(test)]

//! Real Sim owner preparation/custody/boot tests. The other definition owners
//! are inert fixture catalogues: this proves the physical exclusion adapter,
//! not a genuine MindSeal, four-owner IC admission or a region close event.
use super::*;
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, AuthoritativeActionOutput, CanonicalHash, IdempotencyKey, IdempotencyScope,
    PerceivedRef, PreparedAuthoritativeAction, PreparedCausalRef, RequestUid, ScopedIdempotencyKey,
};
use mush_core::action_state::{StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionTxn, BehaviorBinding, BehaviorId, CanonicalId, Effects, Engine, GOD, GameModule, Outbox,
    PrepareCtx, RevisionSet, RulesValue,
};
use mush_testkit::{TestWorld, boot_world};
use serde_json::{Value, json};
use smol_str::SmolStr;
use std::collections::HashMap;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn movement_retains_actual_ic_owner_loaded_generation_and_index_preimages() -> Result {
    for change in 0..3 {
        let mut test = TestWorld::with_modules(vec![Box::new(crate::SimulationModule::new())]);
        test.set_now(100);
        let source = test
            .world()
            .get(GOD)
            .and_then(|o| o.location)
            .ok_or("source")?;
        let destination = test
            .world_mut()
            .create(ObjKind::Room, "destination", GOD, None)?;
        let exit = test
            .world_mut()
            .create(ObjKind::Exit, "north", GOD, Some(source))?;
        test.world_mut().link_exit(exit, Some(destination))?;
        let _ = test.take_module_ops();
        test.world_mut().take_dirty();
        let mut engine = Box::new(test.into_engine());
        let action = engine
            .registry
            .definitions()
            .ok_or("definitions")?
            .current(&mush_core::DefinitionKey {
                owner: OwnerId::new("sim")?,
                kind: DefinitionKind::new("action")?,
                id: DefinitionId::new(mush_core::locomotion::transition::ACTION)?,
            })
            .cloned()
            .ok_or("action")?;
        let request = RequestUid::from_bytes([23; 16]);
        let principal = PrincipalId::Object { id: GOD };
        let invocation = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor: GOD,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([23; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("sim")?,
                    namespace: CanonicalId::new("ic_movement_test")?,
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })?;
        let captured = engine
            .capture_locomotion_command(
                GOD,
                "north",
                &mut Outbox::default(),
                &mut Effects::default(),
            )
            .ok_or("capture")?;
        let (prepared, _) = engine.prepare_captured_locomotion_action(
            captured,
            &action,
            &invocation,
            CommitToken::from_bytes([23; 16]),
        )?;
        let revision = engine.action_state().revision();
        let records = engine.action_state().len();
        let (_, install) = prepared.into_parts();
        // Actual registered slot, same incarnation. This fault injection grants
        // no IC identity or close capability and alters no spatial/Core row.
        let state = engine
            .world
            .module_state_mut::<IcExclusions>()
            .ok_or("IC slot")?;
        match change {
            0 => state.loaded = false,
            1 => state.generation += 1,
            _ => {
                state.active.insert(GOD, CommitToken::from_bytes([24; 16]));
            }
        }
        check!(engine.install_composed_action(install).is_err());
        check_eq!(engine.world.get(GOD).and_then(|o| o.location), Some(source));
        check_eq!(engine.action_state().revision(), revision);
        check_eq!(engine.action_state().len(), records);
        check!(engine.action_state().reservation_lost());
        check_eq!(
            crate::state(&engine.world)
                .ok_or("spatial")?
                .recovery
                .next_seq(crate::data::RECOVERY_LOG),
            0
        );
    }
    Ok(())
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new(kind)?,
        DefinitionId::new(id)?,
        1,
        payload,
    )?)
}
fn reference(owner: &str, descriptor: &DefinitionDescriptor) -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new(owner)?, descriptor.clone())?)
}
struct Catalogue {
    owner: &'static str,
    descriptors: Vec<DefinitionDescriptor>,
    provider: Option<(DefinitionRef, DefinitionRef, DefinitionRef, DefinitionRef)>,
}
impl GameModule for Catalogue {
    fn name(&self) -> &'static str {
        self.owner
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let mut kinds: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for descriptor in &self.descriptors {
            kinds
                .entry(descriptor.kind().clone())
                .or_default()
                .push(descriptor.clone());
        }
        for (kind, candidates) in kinds {
            reg.definition_validator(kind, move |ctx: &DefinitionValidationContext<'_>| {
                if candidates.iter().any(|d| {
                    let mut refs = d.references().to_vec();
                    refs.sort();
                    d.id() == &ctx.definition().reference().key().id
                        && d.payload() == ctx.definition().payload()
                        && refs == ctx.definition().references()
                        && d.is_current() == ctx.definition().is_current()
                }) {
                    Ok(())
                } else {
                    Err("fixture catalog differs".into())
                }
            })?;
        }
        for descriptor in &self.descriptors {
            reg.definition(descriptor.clone())?;
        }
        if let Some((open, close, wrong, outcome)) = &self.provider {
            // Core freezes every action allowed by the real participant, even
            // when this physical-only fixture exercises only Open. A declared
            // Close without an actual provider is an invalid composition.
            reg.action_provider(close.clone(), NoHostClose)?;
            let receipts = receipt_family(reg)?;
            let handle = reg.request_owner_participant::<IcExclusionParticipant>(open.clone())?;
            reg.action_provider(
                open.clone(),
                Probe {
                    receipts: receipts.clone(),
                    handle: handle.clone(),
                    outcome: outcome.clone(),
                },
            )?;
            reg.action_provider(
                wrong.clone(),
                Probe {
                    receipts: receipts.clone(),
                    handle,
                    outcome: outcome.clone(),
                },
            )?;
        }
        Ok(())
    }
}
// This fixture has no genuine region transition. Register the exact allowed
// action, but never manufacture a physical close event to make the roster fit.
struct NoHostClose;
impl ActionProvider for NoHostClose {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Err(ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "physical fixture has no IC host close event",
        )
        .expect("static error"))
    }
}
// Typed evidence for this physical adapter test only. It is derived from the
// actual prepared owner output and never grants a seal, host event or IC claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureReceipt {
    format: u8,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    open: CommitToken,
    unit: ObjId,
    sortie: u64,
    digest: DefinitionDigest,
}
fn receipt_family(
    reg: &mut Registrar<'_>,
) -> std::result::Result<StateFamilyHandle<CommitToken, FixtureReceipt>, ModError> {
    let bad = |e: &dyn std::fmt::Display| ModError::msg(e.to_string());
    reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new("ic_sim_fixture_receipt").map_err(|e| bad(&e))?,
            CanonicalId::new("ic_sim_fixture_receipt.key.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("ic_sim_fixture_receipt.value.v1").map_err(|e| bad(&e))?,
            1,
            CanonicalId::new("ic_sim_fixture_receipt.decode.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("ic_sim_fixture_receipt.validate.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("ic_sim_fixture_receipt.canonical.v1").map_err(|e| bad(&e))?,
        )
        .map_err(|e| bad(&e))?
        .with_limits(32, 4096)
        .map_err(|e| bad(&e))?,
        |key: &CommitToken| Ok(key.to_hex()),
        |value: &Value| {
            let row: FixtureReceipt =
                serde_json::from_value(value.clone()).map_err(|_| "receipt encoding".to_owned())?;
            if serde_json::to_value(&row).map_err(|_| "receipt encoding".to_owned())? != *value {
                return Err("receipt has non-exact fields".into());
            }
            Ok(row)
        },
        |row: &FixtureReceipt| {
            if row.format == 1
                && row.actor.0 != 0
                && row.unit.0 != 0
                && row.sortie != 0
                && row.action.key().owner.as_str() == "tacsim"
                && row.action.key().kind.as_str() == "action"
                && row.action.version() == 1
                && matches!(
                    row.action.key().id.as_str(),
                    "open_ic_mind" | "close_ic_mind"
                )
                && row.digest.as_bytes() != &[0; 32]
            {
                Ok(())
            } else {
                Err("receipt shape".into())
            }
        },
        |row: &FixtureReceipt| serde_json::to_value(row).map_err(|_| "receipt encoding".into()),
    )
}
fn decode_receipt(key: &str, value: &Value) -> Result<FixtureReceipt> {
    let row: FixtureReceipt =
        serde_json::from_value(value.get("value").ok_or("typed value")?.clone())?;
    let expected = json!({"family":{"owner":"tacsim","id":"ic_sim_fixture_receipt"},"key":row.commit.to_hex(),"schema":{"id":"ic_sim_fixture_receipt.value.v1","version":1},"value":row});
    if value != &expected
        || key
            != format!(
                "action-state/v1/ic_sim_fixture_receipt/{}",
                row.commit.to_hex()
            )
    {
        return Err("unexpected typed receipt envelope/key".into());
    }
    Ok(row)
}
struct Probe {
    receipts: StateFamilyHandle<CommitToken, FixtureReceipt>,
    handle: ParticipantHandle<IcExclusionParticipant>,
    outcome: DefinitionRef,
}
impl ActionProvider for Probe {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> std::result::Result<AuthoritativeActionOutput, ActionError> {
        let refusal = || {
            ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "Sim physical adapter refused",
            )
            .expect("static error")
        };
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Params {
            unit: ObjId,
            sortie: u64,
        }
        let p: Params = serde_json::from_value(inv.parameters().clone()).map_err(|_| refusal())?;
        let output = ctx
            .prepare_owner(
                &self.handle,
                IcExclusionRequest::Open(IcIdentity {
                    open: ctx.commit_token(),
                    unit: p.unit,
                    sortie: p.sortie,
                    seal_digest: DefinitionDigest::from_bytes([2; 32]),
                    custody_digest: DefinitionDigest::from_bytes([3; 32]),
                }),
            )
            .map_err(|_| refusal())?;
        txn.insert(
            &self.receipts,
            &output.commit(),
            FixtureReceipt {
                format: 1,
                action: output.action().clone(),
                commit: output.commit(),
                actor: inv.actor(),
                open: output.identity().open,
                unit: output.identity().unit,
                sortie: output.identity().sortie,
                digest: output.digest(),
            },
        )
        .map_err(|_| refusal())?;
        let value = RulesValue::new(
            self.outcome.clone(),
            json!({"commit":output.commit(),"digest":output.digest()}),
        )
        .map_err(|_| refusal())?;
        Ok(AuthoritativeActionOutput::new(value).with_definitions(vec![self.outcome.clone()]))
    }
}
struct Fixture {
    world: TestWorld,
    open: DefinitionRef,
    close: DefinitionRef,
    wrong: DefinitionRef,
    unit: ObjId,
    base: ObjId,
    domain: crate::DomainId,
}
fn setup(extra: Vec<(SmolStr, Value)>, enabled: bool, missing_placement: bool) -> Result<Fixture> {
    let (mut world, registry) = boot_world();
    let field = world.create(ObjKind::Room, "IC field", GOD, None)?;
    let base = world.create(ObjKind::Room, "IC base", GOD, None)?;
    let unit = world.create(ObjKind::Thing, "IC hull", GOD, Some(base))?;
    for (object, behavior) in [
        (field, "sim.domain"),
        (base, "sim.interior"),
        (unit, "sim.participant"),
    ] {
        world.attach_behavior(
            object,
            BehaviorId::new(behavior)?,
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )?;
    }
    let domain = crate::DomainId(1);
    let space = crate::InteriorSpaceRecord {
        object: base,
        anchor: crate::SpaceAnchor::FixedDomain {
            domain,
            anchor_pose: None,
        },
        compartments: BTreeMap::new(),
        rooms: BTreeSet::from([base]),
        egresses: BTreeMap::new(),
    };
    let base_descriptor = ConfiguredIcBase::descriptor(DefinitionId::new("fixture_base")?, &space)?;
    let manifest_descriptor = descriptor(
        "registry_manifest",
        "combat",
        json!({"fixture":"physical-only"}),
    )?;
    let manifest = reference("tacsim", &manifest_descriptor)?;
    let roll = descriptor(
        "owner_participant",
        "commercial_roots",
        json!({"format":1,"protocol":"commercial_retention_v1"}),
    )?;
    let seal = descriptor(
        "owner_participant",
        "ic_sortie",
        json!({"fixture":"inert-seal-catalogue"}),
    )?;
    let custody = descriptor(
        "owner_participant",
        "ic_custody",
        json!({"fixture":"inert-custody-catalogue"}),
    )?;
    let refs = [
        reference("roll", &roll)?,
        reference("tacsim", &seal)?,
        reference("tactics", &custody)?,
        reference("sim", &participant_descriptor()?)?,
    ];
    let config = IcExclusionConfig::new(
        manifest.clone(),
        refs.clone(),
        vec![ConfiguredIcBase::new(
            reference("tacsim", &base_descriptor)?,
            space.clone(),
        )?],
    )?;
    let action = |transition: &str| -> Result<DefinitionDescriptor> {
        let mut dependencies = vec![manifest.clone()];
        dependencies.extend(refs.clone());
        Ok(descriptor(
            "action",
            &format!("{transition}_ic_mind"),
            json!({"format":1,"protocol":"ic_mind_custody_v1","transition":transition}),
        )?
        .with_references(dependencies))
    };
    let open_desc = action("open")?;
    let close_desc = action("close")?;
    let open = reference("tacsim", &open_desc)?;
    let close = reference("tacsim", &close_desc)?;
    let wrong_desc = descriptor("action", "wrong_ic_probe", json!({}))?;
    let wrong = reference("tacsim", &wrong_desc)?;
    let outcome_desc = descriptor("outcome", "ic_probe", json!({}))?;
    let outcome = reference("tacsim", &outcome_desc)?;
    let sim = if enabled {
        crate::SimulationModule::new().with_ic_mind_custody(config)
    } else {
        crate::SimulationModule::new()
    };
    let mut engine = Engine::with_modules(
        world,
        registry,
        vec![
            Box::new(sim),
            Box::new(Catalogue {
                owner: "roll",
                descriptors: vec![roll],
                provider: None,
            }),
            Box::new(Catalogue {
                owner: "tactics",
                descriptors: vec![custody],
                provider: None,
            }),
            Box::new(Catalogue {
                owner: "tacsim",
                descriptors: if enabled {
                    vec![
                        manifest_descriptor,
                        seal,
                        base_descriptor,
                        open_desc,
                        close_desc,
                        wrong_desc,
                        outcome_desc,
                    ]
                } else {
                    // The default roster has no IC participant or action refs.
                    // A retained physical row must be rejected by actual boot,
                    // rather than by a deliberately broken definition graph.
                    vec![manifest_descriptor, seal, base_descriptor]
                },
                provider: enabled.then_some((open.clone(), close.clone(), wrong.clone(), outcome)),
            }),
        ],
    )?;
    let placement = crate::PlacementRecord {
        object: unit,
        parent: crate::SpatialParent::Interior {
            space: base,
            slot: None,
        },
        revision: 1,
        unplaced: None,
    };
    let ground = crate::DomainRecord {
        id: domain,
        host: field,
        driver: crate::passive_definition_ref(),
        status: crate::DomainStatus::Active,
        clock: crate::data::DomainClock::Passive,
        presentation_location: field,
        revision: 1,
        window: None,
    };
    let mut documents = vec![
        (
            format!("domain:{}", domain.0).into(),
            serde_json::to_value(ground)?,
        ),
        (
            format!("space:{}", base.0).into(),
            serde_json::to_value(space)?,
        ),
    ];
    if !missing_placement {
        documents.push((
            format!("placement:{}", unit.0).into(),
            serde_json::to_value(placement)?,
        ));
    }
    documents.extend(extra);
    engine.modules_loaded(
        HashMap::from([(
            "sim".into(),
            ModuleBootData {
                kv: documents.clone(),
                ..Default::default()
            },
        )]),
        &mut Outbox::new(),
        &mut Effects::new(),
    )?;
    // The ordinary fixture bootstrap precedes the composed-action lane. As
    // with the real driver and the Tactics custody fixture, finish that legacy
    // bookkeeping here; never clear effects produced by prepare/stage/take.
    let mut world = TestWorld::from_engine(engine);
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    Ok(Fixture {
        world,
        open,
        close,
        wrong,
        unit,
        base,
        domain,
    })
}
fn invocation(f: &Fixture) -> Result<ActionInvocation> {
    let principal = PrincipalId::Object { id: GOD };
    let request = RequestUid::from_bytes([9; 16]);
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: GOD,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([1; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"unit":f.unit,"sortie":1}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("tacsim")?,
                namespace: CanonicalId::new("ic_probe")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
fn token() -> CommitToken {
    CommitToken::from_bytes([9; 16])
}
fn prepare(
    f: &mut Fixture,
) -> Result<(
    mush_core::composed::PreparedComposedTxn,
    PreparedAuthoritativeAction,
    ActionInvocation,
)> {
    let inv = invocation(f)?;
    let (p, o) = f
        .world
        .engine_mut()
        .prepare_registered_composed_authoritative_action(&f.open, &inv, token())?;
    Ok((p, o, inv))
}
fn stage(
    f: &mut Fixture,
    p: mush_core::composed::PreparedComposedTxn,
    o: PreparedAuthoritativeAction,
    inv: &ActionInvocation,
) -> Result {
    let (outcome, resolution, mut definitions, mut witnesses, mut durable_outbox, semantic_events) =
        o.into_parts();
    definitions.extend([f.open.clone(), outcome.schema().clone()]);
    definitions.sort();
    definitions.dedup();
    if witnesses.is_empty() {
        witnesses.push(inv.principal().clone());
    }
    witnesses.sort();
    witnesses.dedup();
    durable_outbox.sort_by_key(|o| o.ordinal());
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token: token(),
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([3; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([4; 32]),
        action: f.open.clone(),
        provider: p.owner().clone(),
        cause: PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: p.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token())),
        semantic_events,
        eligible_witnesses: witnesses,
        durable_outbox,
    })?;
    let guards = f
        .world
        .registry()
        .server_guard_set_builder(RevisionSet::default())?
        .finish()?;
    f.world
        .engine_mut()
        .stage_authoritative_action(p, guards, bundle)?;
    Ok(())
}
fn row_writes(p: &mush_core::composed::PreparedComposedTxn) -> Result<Vec<(SmolStr, Value)>> {
    let mut physical = Vec::new();
    let mut receipt = None;
    for write in p.authoritative_replay_writes() {
        match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "sim" => physical.push((key.to_string().into(), value)),
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "tacsim" => {
                if receipt
                    .replace(decode_receipt(key.as_str(), &value)?)
                    .is_some()
                {
                    return Err("duplicate typed fixture receipt".into());
                }
            }
            _ => return Err("unexpected composed fixture write".into()),
        }
    }
    let receipt = receipt.ok_or("missing typed fixture receipt")?;
    if physical.len() != 1 {
        return Err("wrong physical image count".into());
    }
    let (key, value) = physical
        .iter()
        .find(|(key, _): &&(SmolStr, Value)| key.starts_with(PREFIX))
        .ok_or("physical history")?;
    let record: Record = serde_json::from_value(value.clone())?;
    record.validate()?;
    let action = record
        .closed
        .as_ref()
        .map(|v| &v.action)
        .unwrap_or(&record.open.action);
    let commit = record
        .closed
        .as_ref()
        .map(|v| v.operation)
        .unwrap_or(record.open.identity.open);
    if key.as_str() != record.key()
        || receipt.format != 1
        || &receipt.action != action
        || receipt.commit != commit
        || receipt.actor != record.open.dispatcher
        || receipt.open != record.open.identity.open
        || receipt.unit != record.open.identity.unit
        || receipt.sortie != record.open.identity.sortie
        || receipt.digest != record.digest()?
    {
        return Err("typed receipt differs from actual owner image".into());
    }
    Ok(physical)
}
#[test]
fn real_owner_is_inert_until_install_and_only_the_hull_is_excluded() -> Result {
    let mut f = setup(vec![], true, false)?;
    let inv = invocation(&f)?;
    let close = f.close.clone();
    let refused = f
        .world
        .engine_mut()
        .prepare_registered_composed_authoritative_action(
            &close,
            &inv,
            CommitToken::from_bytes([8; 16]),
        );
    check!(
        matches!(refused, Err(mush_core::action::ActionProviderPrepareError::Provider{source,..})
        if source.code() == ActionErrorCode::ProviderRejected)
    );
    check!(identity_of(f.world.world(), f.unit).is_none());
    let (p, o, inv) = prepare(&mut f)?;
    let persisted = row_writes(&p)?;
    check_eq!(persisted.len(), 1);
    check!(identity_of(f.world.world(), f.unit).is_none());
    stage(&mut f, p, o, &inv)?;
    check!(identity_of(f.world.world(), f.unit).is_none());
    let held = f
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged image")?;
    check!(identity_of(f.world.world(), f.unit).is_none());
    let (_, installation) = held.into_parts();
    f.world.engine_mut().install_composed_action(installation)?;
    check_eq!(
        identity_of(f.world.world(), f.unit).map(|i| i.open),
        Some(token())
    );
    check!(crate::deployed::refuse_if_custodied(f.world.world(), f.unit).is_err());
    check!(crate::deployed::mark_of(f.world.world(), f.unit).is_none());
    check!(crate::deployed::excluded(
        f.world.world(),
        crate::deployed::Shown::Unit(f.unit)
    ));
    check!(!crate::deployed::excluded(
        f.world.world(),
        crate::deployed::Shown::Unit(f.base)
    ));
    check!(!crate::deployed::excluded(
        f.world.world(),
        crate::deployed::Shown::Domain(f.domain)
    ));
    let state = crate::state(f.world.world()).ok_or("spatial")?;
    check!(
        crate::transition::plan(
            state,
            f.world.world(),
            &crate::transition::Transition::Place {
                obj: f.unit,
                parent: crate::SpatialParent::Interior {
                    space: f.base,
                    slot: None
                }
            }
        )
        .is_err()
    );
    let cold = setup(persisted.clone(), true, false)?;
    check!(identity_of(cold.world.world(), cold.unit).is_some());
    check!(crate::deployed::deployed_units(cold.world.world()).is_empty());
    check!(
        setup(persisted.clone(), true, true).is_err(),
        "missing held placement must not be healed"
    );
    check!(
        setup(persisted, false, false).is_err(),
        "disabled mode cannot discard durable IC exclusion"
    );
    Ok(())
}
#[test]
fn actual_stage_and_take_recheck_placement_base_and_crossed_action() -> Result {
    for after_stage in [false, true] {
        for field in [0, 1, 2] {
            let mut f = setup(vec![], true, false)?;
            let (p, o, inv) = prepare(&mut f)?;
            if after_stage {
                stage(&mut f, p, o, &inv)?;
                drift(&mut f, field)?;
                check!(f.world.engine_mut().take_staged_action().is_err());
            } else {
                drift(&mut f, field)?;
                check!(stage(&mut f, p, o, &inv).is_err());
            }
            check!(identity_of(f.world.world(), f.unit).is_none());
        }
    }
    let mut f = setup(vec![], true, false)?;
    let inv = invocation(&f)?;
    check!(
        f.world
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&f.wrong, &inv, token())
            .is_err()
    );
    Ok(())
}
fn drift(f: &mut Fixture, field: u8) -> Result {
    let state = f
        .world
        .world_mut()
        .module_state_mut::<crate::SpatialState>()
        .ok_or("spatial")?;
    match field {
        0 => {
            state
                .placements
                .get_mut(&f.unit)
                .ok_or("placement")?
                .revision += 1
        }
        1 => state.spaces.get_mut(&f.base).ok_or("space")?.anchor = crate::SpaceAnchor::Unanchored,
        _ => state.domains.get_mut(&f.domain).ok_or("domain")?.status = crate::DomainStatus::Paused,
    }
    Ok(())
}
#[test]
fn exact_base_and_strict_history_refuse_ambiguous_data() -> Result {
    let mut f = setup(vec![], true, false)?;
    let (p, _, _) = prepare(&mut f)?;
    let rows = row_writes(&p)?;
    // This is shape-only cold input, never an invented close capability.
    let (_, value) = rows.first().ok_or("physical row")?;
    let original: Record = serde_json::from_value(value.clone())?;
    let mut crossed = original.clone();
    crossed.open.identity.open = CommitToken::from_bytes([8; 16]);
    crossed.open.identity.unit = ObjId(99);
    crossed.open.placement.object = ObjId(99);
    check!(
        IcExclusions::reconstruct([original.clone(), crossed]).is_err(),
        "sortie cannot be reused for another hull"
    );
    let mut missing = value.clone();
    missing.as_object_mut().ok_or("record")?.remove("closed");
    check!(serde_json::from_value::<Record>(missing).is_err());
    let mut unknown = value.clone();
    unknown
        .as_object_mut()
        .ok_or("record")?
        .insert("receipt_authority".into(), json!(true));
    check!(serde_json::from_value::<Record>(unknown).is_err());
    let mut stale = original.open.base.space.clone();
    stale.rooms.clear();
    check!(ConfiguredIcBase::new(original.open.base.definition.clone(), stale).is_err());
    let mut f = setup(vec![], true, false)?;
    f.world
        .world_mut()
        .module_state_mut::<crate::SpatialState>()
        .ok_or("spatial")?
        .placements
        .get_mut(&f.unit)
        .ok_or("placement")?
        .parent = crate::SpatialParent::Unplaced;
    check!(prepare(&mut f).is_err());
    check!(
        setup(vec![], false, false).is_ok(),
        "default spatial boot remains usable"
    );
    Ok(())
}

#[test]
fn retained_plans_and_domain_retirement_cannot_cross_actual_ic_custody() -> Result {
    let mut f = setup(vec![], true, false)?;
    let held_before = crate::transition::plan(
        crate::state(f.world.world()).ok_or("spatial")?,
        f.world.world(),
        &crate::transition::Transition::Place {
            obj: f.unit,
            parent: crate::SpatialParent::Interior {
                space: f.base,
                slot: None,
            },
        },
    )?;
    let (p, o, inv) = prepare(&mut f)?;
    stage(&mut f, p, o, &inv)?;
    let held = f.world.engine_mut().take_staged_action()?.ok_or("staged")?;
    let (_, installation) = held.into_parts();
    f.world.engine_mut().install_composed_action(installation)?;
    let normal_retire = crate::transition::plan(
        crate::state(f.world.world()).ok_or("spatial")?,
        f.world.world(),
        &crate::transition::Transition::Retire { domain: f.domain },
    );
    check!(matches!(normal_retire,
        Err(crate::transition::SpatialError::DomainReferenced(domain, n)) if domain == f.domain && n > 0));
    for plan in [
        held_before,
        crate::transition::Plan {
            retire: vec![f.domain],
            audit: json!({"op":"retire"}),
            ..Default::default()
        },
    ] {
        assert_ic_plan_refused_without_effect(&mut f, plan)?;
    }
    // The index is owner-private and all producers maintain it. A deliberately
    // broken internal fixture must still fail closed, never skip its held hull.
    f.world
        .world_mut()
        .module_state_mut::<IcExclusions>()
        .ok_or("exclusion")?
        .records
        .remove(&token())
        .ok_or("held row")?;
    assert_ic_plan_refused_without_effect(&mut f, crate::transition::Plan::default())?;
    Ok(())
}
fn assert_ic_plan_refused_without_effect(f: &mut Fixture, plan: crate::transition::Plan) -> Result {
    let mut spatial = std::mem::take(
        f.world
            .world_mut()
            .module_state_mut::<crate::SpatialState>()
            .ok_or("spatial")?,
    );
    let before = spatial.clone();
    let location = f.world.world().get(f.unit).ok_or("hull")?.location;
    let mut out = Outbox::new();
    let result = crate::transition::apply(&mut spatial, f.world.world_mut(), &mut out, plan);
    check!(
        matches!(result, Err(crate::transition::SpatialError::IcCustody(unit)) if unit == f.unit)
    );
    check!(
        spatial == before,
        "refused IC transition changed spatial state"
    );
    check_eq!(
        f.world.world().get(f.unit).ok_or("hull")?.location,
        location
    );
    check!(
        out.events().is_empty(),
        "refused IC transition emitted persistence/output"
    );
    *f.world
        .world_mut()
        .module_state_mut::<crate::SpatialState>()
        .ok_or("spatial")? = spatial;
    Ok(())
}
