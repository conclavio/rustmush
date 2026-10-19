//! Actual Tactics preparation, custody, installation and cold decoding. The
//! fixture's Tacsim provider supplies structural physical requests only: these
//! tests do not mint a real seal, host event, paid claim or four-owner union.
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
    ActionTxn, CanonicalId, DefinitionValidationContext, Effects, Engine, GOD, GameModule, Outbox,
    PrepareCtx, Registry, RevisionSet, RulesValue,
};
use mush_testkit::{TestWorld, boot_world};
use serde_json::{Value, json};
use smol_str::SmolStr;
use std::collections::HashMap;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn descriptor(kind: &str, id: &str, value: Value) -> Result<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new(kind)?,
        DefinitionId::new(id)?,
        1,
        value,
    )?)
}
fn reference(owner: &str, d: &DefinitionDescriptor) -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new(owner)?, d.clone())?)
}
#[derive(Clone)]
struct Catalogue {
    owner: &'static str,
    descriptors: Vec<DefinitionDescriptor>,
    providers: Option<[DefinitionRef; 4]>,
}
impl GameModule for Catalogue {
    fn name(&self) -> &'static str {
        self.owner
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let mut kinds: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for d in &self.descriptors {
            kinds.entry(d.kind().clone()).or_default().push(d.clone());
        }
        for (kind, expected) in kinds {
            reg.definition_validator(kind, move |ctx: &DefinitionValidationContext<'_>| {
                let actual = ctx.definition();
                if expected.iter().any(|d| {
                    let mut refs = d.references().to_vec();
                    refs.sort();
                    d.id() == &actual.reference().key().id
                        && d.version() == actual.reference().version()
                        && d.payload() == actual.payload()
                        && refs == actual.references()
                        && d.is_current() == actual.is_current()
                }) {
                    Ok(())
                } else {
                    Err("fixture descriptor differs".into())
                }
            })?;
        }
        for d in &self.descriptors {
            reg.definition(d.clone())?;
        }
        if let Some([open, close, wrong, outcome]) = &self.providers {
            let receipts = receipt_family(reg)?;
            let h = reg.request_owner_participant::<IcCustodyParticipant>(open.clone())?;
            reg.action_provider(
                open.clone(),
                Probe {
                    receipts: receipts.clone(),
                    handle: h.clone(),
                    close: false,
                    outcome: outcome.clone(),
                },
            )?;
            reg.action_provider(
                wrong.clone(),
                Probe {
                    receipts: receipts.clone(),
                    handle: h,
                    close: false,
                    outcome: outcome.clone(),
                },
            )?;
            let h = reg.request_owner_participant::<IcCustodyParticipant>(close.clone())?;
            reg.action_provider(
                close.clone(),
                Probe {
                    receipts: receipts.clone(),
                    handle: h,
                    close: true,
                    outcome: outcome.clone(),
                },
            )?;
        }
        Ok(())
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
            StateFamilyId::new("ic_tactics_fixture_receipt").map_err(|e| bad(&e))?,
            CanonicalId::new("ic_tactics_fixture_receipt.key.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("ic_tactics_fixture_receipt.value.v1").map_err(|e| bad(&e))?,
            1,
            CanonicalId::new("ic_tactics_fixture_receipt.decode.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("ic_tactics_fixture_receipt.validate.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("ic_tactics_fixture_receipt.canonical.v1").map_err(|e| bad(&e))?,
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
    let expected = json!({"family":{"owner":"tacsim","id":"ic_tactics_fixture_receipt"},"key":row.commit.to_hex(),"schema":{"id":"ic_tactics_fixture_receipt.value.v1","version":1},"value":row});
    if value != &expected
        || key
            != format!(
                "action-state/v1/ic_tactics_fixture_receipt/{}",
                row.commit.to_hex()
            )
    {
        return Err("unexpected typed receipt envelope/key".into());
    }
    Ok(row)
}
struct Probe {
    receipts: StateFamilyHandle<CommitToken, FixtureReceipt>,
    handle: ParticipantHandle<IcCustodyParticipant>,
    close: bool,
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
        let error = || {
            ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                "Tactics IC fixture refused",
            )
            .expect("static refusal")
        };
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Params {
            unit: ObjId,
            revision: u64,
            sortie: u64,
        }
        let p: Params = serde_json::from_value(inv.parameters().clone()).map_err(|_| error())?;
        let request = if self.close {
            IcCustodyRequest::Close {
                open: open_token(),
                event: IcEventRef {
                    sequence: 2,
                    digest: DefinitionDigest::from_bytes([5; 32]),
                },
                conditions: UnitMutationPlan::new(
                    vec![UnitMutationOp::SetCell {
                        addr: crate::CellAddr::Unit {
                            cell: crate::CellId::new("heat").map_err(|_| error())?,
                        },
                        value: 9,
                    }],
                    Some(p.revision),
                    None,
                )
                .map_err(|_| error())?,
            }
        } else {
            IcCustodyRequest::Open {
                unit: p.unit,
                sortie: p.sortie,
                seal_digest: DefinitionDigest::from_bytes([4; 32]),
            }
        };
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
                actor: inv.actor(),
                open: output.open(),
                unit: output.unit(),
                sortie: output.sortie(),
                digest: output.digest(),
            },
        )
        .map_err(|_| error())?;
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(
                self.outcome.clone(),
                json!({"commit":output.commit(),"open":output.open(),"digest":output.digest()}),
            )
            .map_err(|_| error())?,
        )
        .with_definitions(vec![self.outcome.clone()]))
    }
}
#[derive(Clone)]
struct NeverReplace;
impl crate::component_replacement::ReplacementPolicy for NeverReplace {
    type Request = ();
    fn authorize(
        &self,
        _: &OwnerPrepareContext<'_>,
        _: &(),
        _: &TacticalUnit,
    ) -> std::result::Result<crate::component_replacement::ReplacementChoice, ParticipantError>
    {
        Err(ParticipantError::refused(
            "fixture replacement has no business authority",
        ))
    }
}

#[derive(Clone)]
struct Rules {
    roll: Catalogue,
    tacsim: Catalogue,
    spec: crate::UnitSpec,
    custody: IcCustodyConfig,
    exclusion: mush_sim::ic_exclusion::IcExclusionConfig,
    open: DefinitionRef,
    close: DefinitionRef,
    wrong: DefinitionRef,
}
impl Rules {
    fn new(base: ObjId) -> Result<Self> {
        let schema = descriptor("schema", "physical_rules", json!({}))?;
        let component = descriptor("component", "physical_core", json!({}))?;
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
                id: crate::ComponentId::new("core")?,
                definition: reference("tacsim", &component)?,
                install: crate::InstallPoint::Unit,
                condition: crate::ConditionId::new("ok")?,
                cells: vec![],
                asset: None,
            }],
            capabilities: vec![],
            mobility: None,
            rules: RulesValue::new(reference("tacsim", &schema)?, json!({}))?,
        };
        let unit = body
            .descriptor(
                DefinitionKind::new("unit")?,
                DefinitionId::new("physical_hull")?,
                1,
            )?
            .as_current();
        let spec = body.with_definition(reference("tacsim", &unit)?);
        let manifest_desc = descriptor(
            "registry_manifest",
            "combat",
            json!({"fixture":"physical adapter only"}),
        )?;
        let manifest = reference("tacsim", &manifest_desc)?;
        let roll = descriptor(
            "owner_participant",
            "commercial_roots",
            json!({"format":1,"protocol":"commercial_retention_v1"}),
        )?;
        let seal = descriptor(
            "owner_participant",
            "ic_sortie",
            json!({"fixture":"inert seal descriptor"}),
        )?;
        let participants = [
            reference("roll", &roll)?,
            reference("tacsim", &seal)?,
            reference("tactics", &participant_descriptor()?)?,
            reference("sim", &mush_sim::ic_exclusion::participant_descriptor()?)?,
        ];
        let space = mush_sim::InteriorSpaceRecord {
            object: base,
            anchor: mush_sim::SpaceAnchor::FixedDomain {
                domain: mush_sim::DomainId(1),
                anchor_pose: None,
            },
            compartments: BTreeMap::new(),
            rooms: BTreeSet::from([base]),
            egresses: BTreeMap::new(),
        };
        let base_desc = mush_sim::ic_exclusion::ConfiguredIcBase::descriptor(
            DefinitionId::new("physical_base")?,
            &space,
        )?;
        let exclusion = mush_sim::ic_exclusion::IcExclusionConfig::new(
            manifest.clone(),
            participants.clone(),
            vec![mush_sim::ic_exclusion::ConfiguredIcBase::new(
                reference("tacsim", &base_desc)?,
                space,
            )?],
        )?;
        let custody = IcCustodyConfig::new(manifest.clone(), participants.clone())?;
        let action = |transition: &str| -> Result<DefinitionDescriptor> {
            Ok(descriptor(
                "action",
                &format!("{transition}_ic_mind"),
                json!({"format":1,"protocol":"ic_mind_custody_v1","transition":transition}),
            )?
            .with_references(
                std::iter::once(manifest.clone())
                    .chain(participants.iter().cloned())
                    .collect(),
            ))
        };
        let open_desc = action("open")?;
        let close_desc = action("close")?;
        let wrong_desc = descriptor("action", "wrong_ic_physical", json!({}))?;
        let outcome_desc = descriptor("outcome", "ic_physical", json!({}))?;
        let open = reference("tacsim", &open_desc)?;
        let close = reference("tacsim", &close_desc)?;
        let wrong = reference("tacsim", &wrong_desc)?;
        let outcome = reference("tacsim", &outcome_desc)?;
        Ok(Self {
            roll: Catalogue {
                owner: "roll",
                descriptors: vec![roll],
                providers: None,
            },
            tacsim: Catalogue {
                owner: "tacsim",
                descriptors: vec![
                    schema,
                    component,
                    unit,
                    manifest_desc,
                    seal,
                    base_desc,
                    open_desc,
                    close_desc,
                    wrong_desc,
                    outcome_desc,
                ],
                providers: Some([open.clone(), close.clone(), wrong.clone(), outcome]),
            },
            spec,
            custody,
            exclusion,
            open,
            close,
            wrong,
        })
    }
    fn modules(&self, enabled: bool) -> Result<Vec<Box<dyn GameModule>>> {
        let mut catalog = crate::UnitCatalogBuilder::new();
        catalog.add(
            self.spec.clone(),
            &crate::MobilityVocabulary::base().freeze(),
        )?;
        let tactics = crate::TacticsModule::with_catalog(catalog.freeze());
        let mut tacsim = self.tacsim.clone();
        let (sim, tactics) = if enabled {
            // Each boot gets fresh captured registry cells, never a previous
            // Engine's already-initialized configuration.
            let custody = IcCustodyConfig::new(
                self.custody.manifest.clone(),
                self.custody.participants.clone(),
            )?;
            (
                mush_sim::SimulationModule::new().with_ic_mind_custody(self.exclusion.clone()),
                // The existing exact replacement adapter shares this actual
                // UnitStore under a disjoint action set. Core still refuses
                // two participants for the slot in any one invocation.
                tactics
                    .with_component_replacement(
                        NeverReplace,
                        self.wrong.clone(),
                        crate::ComponentId::new("core")?,
                        BTreeSet::from([self.spec.definition.clone()]),
                    )
                    .with_ic_mind_custody(custody),
            )
        } else {
            tacsim.providers = None;
            tacsim.descriptors.retain(|d| d.kind().as_str() != "action");
            (mush_sim::SimulationModule::new(), tactics)
        };
        Ok(vec![
            Box::new(sim),
            Box::new(self.roll.clone()),
            Box::new(tacsim),
            Box::new(tactics),
        ])
    }
}
struct Fixture {
    world: TestWorld,
    rules: Rules,
    hull: ObjId,
}
fn setup() -> Result<Fixture> {
    let (mut world, registry) = boot_world();
    let base = world.create(ObjKind::Room, "Physical IC base", GOD, None)?;
    let hull = world.create(ObjKind::Thing, "Physical IC hull", GOD, Some(base))?;
    let rules = Rules::new(base)?;
    let mut engine = Engine::with_modules(world, registry, rules.modules(true)?)?;
    engine.modules_loaded(HashMap::new(), &mut Outbox::new(), &mut Effects::new())?;
    let mut world = TestWorld::from_engine(engine);
    world.run(GOD, &format!("@unit/create #{}=physical_hull", hull.0));
    assert!(
        world
            .world()
            .module_state::<UnitStore>()
            .is_some_and(|s| s.units.contains_key(&hull)),
        "actual unit command must create hull"
    );
    let _ = world.world_mut().take_dirty();
    let _ = world.take_module_ops();
    Ok(Fixture { world, rules, hull })
}
fn open_token() -> CommitToken {
    CommitToken::from_bytes([10; 16])
}
fn close_token() -> CommitToken {
    CommitToken::from_bytes([11; 16])
}
fn stored(f: &Fixture) -> UnitStore {
    f.world
        .world()
        .module_state::<UnitStore>()
        .expect("actual tactics")
        .clone()
}
fn invocation(f: &Fixture) -> Result<ActionInvocation> {
    let request = RequestUid::from_bytes([7; 16]);
    let principal = PrincipalId::Object { id: GOD };
    let revision = stored(f).units.get(&f.hull).ok_or("hull")?.revision();
    Ok(ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: GOD,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([1; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({"unit":f.hull,"revision":revision,"sortie":1}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("tacsim")?,
                namespace: CanonicalId::new("ic_physical")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?)
}
type Prepared = (
    mush_core::composed::PreparedComposedTxn,
    PreparedAuthoritativeAction,
    ActionInvocation,
);
fn prepare(f: &mut Fixture, close: bool) -> Result<Prepared> {
    let inv = invocation(f)?;
    let action = if close { &f.rules.close } else { &f.rules.open };
    let token = if close { close_token() } else { open_token() };
    let (p, o) = f
        .world
        .engine_mut()
        .prepare_registered_composed_authoritative_action(action, &inv, token)?;
    Ok((p, o, inv))
}
fn stage(f: &mut Fixture, close: bool, (p, o, inv): Prepared) -> Result {
    let token = if close { close_token() } else { open_token() };
    let action = if close {
        f.rules.close.clone()
    } else {
        f.rules.open.clone()
    };
    let (outcome, resolution, mut definitions, mut witnesses, mut durable_outbox, semantic_events) =
        o.into_parts();
    definitions.extend([action.clone(), outcome.schema().clone()]);
    definitions.sort();
    definitions.dedup();
    if witnesses.is_empty() {
        witnesses.push(inv.principal().clone());
    }
    witnesses.sort();
    witnesses.dedup();
    durable_outbox.sort_by_key(|o| o.ordinal());
    let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
        token,
        request: inv.request(),
        idempotency: inv.idempotency().clone(),
        ingress_hash: CanonicalHash::from_bytes([2; 32]),
        request_hash: inv.request_hash(),
        plan_hash: CanonicalHash::from_bytes([3; 32]),
        action,
        provider: p.owner().clone(),
        cause: PreparedCausalRef::new(inv.request(), None, 0)?,
        definitions,
        writes: p.authoritative_replay_writes(),
        outcome,
        resolution: resolution.map(|r| r.bind(token)),
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
fn install(f: &mut Fixture) -> Result {
    let held = f
        .world
        .engine_mut()
        .take_staged_action()?
        .ok_or("staged physical image")?;
    let (_, installation) = held.into_parts();
    f.world.engine_mut().install_composed_action(installation)?;
    Ok(())
}
fn rows(p: &mush_core::composed::PreparedComposedTxn) -> Result<Vec<(SmolStr, Value)>> {
    let mut physical = Vec::new();
    let mut receipt = None;
    for write in p.authoritative_replay_writes() {
        match write {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } if namespace.as_str() == "tactics" => physical.push((key.to_string().into(), value)),
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
    if physical.len() != 2 {
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
        .unwrap_or(record.open.operation);
    if key.as_str() != record.key()
        || receipt.format != 1
        || &receipt.action != action
        || receipt.commit != commit
        || receipt.actor != record.open.dispatcher
        || receipt.open != record.open.operation
        || receipt.unit != record.open.unit
        || receipt.sortie != record.open.sortie
        || receipt.digest != record.digest()?
    {
        return Err("typed receipt differs from actual owner image".into());
    }
    if !physical
        .iter()
        .any(|(key, _)| key.as_str() == format!("unit:{}", record.open.unit.0))
    {
        return Err("missing exact unit image".into());
    }
    Ok(physical)
}
fn cold(f: &Fixture, rows: Vec<(SmolStr, Value)>, enabled: bool) -> Result<Engine> {
    // Fresh Rules also recreates the actual Sim configuration registry cell.
    let base = f
        .world
        .world()
        .get(f.hull)
        .and_then(|o| o.location)
        .ok_or("base")?;
    let rules = Rules::new(base)?;
    let mut e = Engine::with_modules(
        f.world.world().clone(),
        Registry::new(),
        rules.modules(enabled)?,
    )?;
    e.modules_loaded(
        HashMap::from([(
            "tactics".into(),
            ModuleBootData {
                kv: rows,
                ..Default::default()
            },
        )]),
        &mut Outbox::new(),
        &mut Effects::new(),
    )?;
    Ok(e)
}

#[test]
fn actual_open_retains_unit_bytes_and_fences_mutations_until_exact_close() -> Result {
    let mut f = setup()?;
    let before = stored(&f);
    let prepared = prepare(&mut f, false)?;
    let persisted = rows(&prepared.0)?;
    assert_eq!(persisted.len(), 2);
    assert_eq!(stored(&f), before);
    let unit = before.units.get(&f.hull).ok_or("unit")?;
    let schema = before.record_definition.as_ref().ok_or("schema")?;
    let unit_value = persisted
        .iter()
        .find(|(k, _)| k.as_str() == format!("unit:{}", f.hull.0))
        .ok_or("unit write")?
        .1
        .clone();
    assert_eq!(
        unit_value,
        encode_unit_document(schema, unit, None)?.0,
        "Open preserves old unit codec bytes"
    );
    assert!(unit_value.get("ic_custody").is_none());
    stage(&mut f, false, prepared)?;
    assert_eq!(stored(&f), before);
    let held = f.world.engine_mut().take_staged_action()?.ok_or("held")?;
    assert_eq!(stored(&f), before);
    let (_, installation) = held.into_parts();
    f.world.engine_mut().install_composed_action(installation)?;
    assert!(stored(&f).is_ic_custodied(f.hull));
    assert!(stored(&f).deployed.is_empty());
    assert!(mush_sim::deployed::mark_of(f.world.world(), f.hull).is_none());
    // This deliberately tests the owner-local fallback with no Sim exclusion
    // installed. Such a partial union is not an activatable full IC world.
    let held = stored(&f);
    assert!(refuse_mutation(f.world.world(), f.hull).is_err());
    let plan = UnitMutationPlan::new(
        vec![UnitMutationOp::SetCell {
            addr: crate::CellAddr::Unit {
                cell: crate::CellId::new("heat")?,
            },
            value: 17,
        }],
        Some(unit.revision()),
        None,
    )?;
    f.world.run(
        GOD,
        &format!(
            "@unit/damage #{}={}",
            f.hull.0,
            serde_json::to_string(&plan)?
        ),
    );
    f.world.run(GOD, &format!("@unit/remove #{}", f.hull.0));
    assert_eq!(stored(&f), held);
    let reopened = cold(&f, persisted.clone(), true)?;
    assert!(
        reopened
            .world
            .module_state::<UnitStore>()
            .ok_or("cold slot")?
            .is_ic_custodied(f.hull)
    );
    assert!(cold(&f, persisted.clone(), false).is_err());
    let mut missing = persisted.clone();
    missing.retain(|(k, _)| !k.starts_with("unit:"));
    assert!(cold(&f, missing, true).is_err());
    let prepared = prepare(&mut f, true)?;
    let closed = rows(&prepared.0)?;
    assert_eq!(stored(&f), held);
    stage(&mut f, true, prepared)?;
    assert_eq!(stored(&f), held);
    install(&mut f)?;
    let after = stored(&f);
    assert!(!after.is_ic_custodied(f.hull));
    assert_eq!(after.ic.rows.len(), 1);
    assert_eq!(
        after.units.get(&f.hull).ok_or("after")?.revision(),
        unit.revision() + 1
    );
    assert_eq!(
        after
            .units
            .get(&f.hull)
            .ok_or("after")?
            .cell(&crate::CellAddr::Unit {
                cell: crate::CellId::new("heat")?
            })
            .ok_or("heat")?
            .current,
        9
    );
    assert!(refuse_mutation(f.world.world(), f.hull).is_ok());
    let reopened = cold(&f, closed, true)?;
    assert!(
        !reopened
            .world
            .module_state::<UnitStore>()
            .ok_or("cold slot")?
            .is_ic_custodied(f.hull)
    );
    assert!(
        prepare(&mut f, true).is_err(),
        "a decoded repeated close is not a new event"
    );
    Ok(())
}
#[test]
fn actual_preparation_stage_take_and_bind_refuse_changed_owner_images() -> Result {
    for at in 0..3 {
        let mut f = setup()?;
        let prepared = prepare(&mut f, false)?;
        if at == 0 {
            drift(&mut f)?;
            assert!(stage(&mut f, false, prepared).is_err());
        } else {
            stage(&mut f, false, prepared)?;
            if at == 1 {
                drift(&mut f)?;
                assert!(f.world.engine_mut().take_staged_action().is_err());
            } else {
                let held = f.world.engine_mut().take_staged_action()?.ok_or("held")?;
                let (_, installation) = held.into_parts();
                drift(&mut f)?;
                let before = stored(&f);
                assert!(
                    f.world
                        .engine_mut()
                        .install_composed_action(installation)
                        .is_err()
                );
                assert_eq!(stored(&f), before);
            }
        }
        assert!(!stored(&f).is_ic_custodied(f.hull));
    }
    let mut f = setup()?;
    let inv = invocation(&f)?;
    assert!(
        f.world
            .engine_mut()
            .prepare_registered_composed_authoritative_action(&f.rules.wrong, &inv, open_token())
            .is_err()
    );
    let prepared = prepare(&mut f, false)?;
    stage(&mut f, false, prepared)?;
    mush_sim::deployed::mark(
        f.world.world_mut(),
        f.hull,
        mush_sim::DeployedMark::new(9, "other-arena"),
    );
    assert!(f.world.engine_mut().take_staged_action().is_err());
    Ok(())
}
fn drift(f: &mut Fixture) -> Result {
    let hull = f.hull;
    let store = f
        .world
        .world_mut()
        .module_state_mut::<UnitStore>()
        .ok_or("store")?;
    store
        .records
        .iter_mut()
        .find(|r| r.claims_object(hull))
        .ok_or("row")?
        .payload_digest = None;
    Ok(())
}
#[test]
fn strict_history_and_condition_only_close_preserve_old_ooc_boundary() -> Result {
    let mut f = setup()?;
    let prepared = prepare(&mut f, false)?;
    let persisted = rows(&prepared.0)?;
    stage(&mut f, false, prepared)?;
    install(&mut f)?;
    let record: Record = serde_json::from_value(
        persisted
            .iter()
            .find(|(k, _)| k.starts_with(PREFIX))
            .ok_or("history")?
            .1
            .clone(),
    )?;
    let mut value = serde_json::to_value(&record)?;
    value.as_object_mut().ok_or("object")?.remove("closed");
    assert!(serde_json::from_value::<Record>(value).is_err());
    let mut value = serde_json::to_value(&record)?;
    value
        .as_object_mut()
        .ok_or("object")?
        .insert("receipt_authority".into(), json!(true));
    assert!(serde_json::from_value::<Record>(value).is_err());
    let mut crossed = record.clone();
    crossed.open.operation = CommitToken::from_bytes([22; 16]);
    crossed.open.unit = ObjId(900);
    assert!(
        IcCustodies::reconstruct([record, crossed]).is_err(),
        "sortie IDs are unique"
    );
    let state = stored(&f);
    let unit = state.units.get(&f.hull).ok_or("unit")?;
    capacity(state.record_definition.as_ref().ok_or("schema")?, unit)?;
    let asset = UnitMutationPlan::new(
        vec![UnitMutationOp::SetComponentAsset {
            component: crate::ComponentId::new("core")?,
            asset: Some(ObjId(900)),
        }],
        Some(unit.revision()),
        None,
    )?;
    assert!(
        close_unit(unit, &asset, f.world.world().now()).is_err(),
        "close cannot replace an asset"
    );
    let stale = UnitMutationPlan::new(vec![], Some(unit.revision() + 1), None)?;
    assert!(close_unit(unit, &stale, 0).is_err());
    let absent = UnitMutationPlan::new(vec![], None, None)?;
    assert!(close_unit(unit, &absent, 0).is_err());
    let mut disabled = cold(&f, vec![], false)?;
    assert!(
        disabled
            .world
            .module_state_mut::<UnitStore>()
            .ok_or("default store")?
            .ic
            .rows
            .is_empty()
    );
    Ok(())
}
