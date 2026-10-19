//! Actual Engine registration of the real Economy module and its existing
//! InitialFilingParticipant. Other owners provide strict inert definitions and
//! providers only: there is no Roll business state, certificate, payment,
//! fabricated replay latch, source grant, or successful action preparation.
use crate::registry_filing::{InitialFilingParticipant, InitialFilingPolicy};
use crate::{AccountId, Amount, CatalogRef, EconomyModule, EconomyStore};
use mush_core::{
    Engine, GameModule, ModError, Registrar, Registry, World,
    action::{ActionError, ActionInvocation, ActionProvider},
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
        DefinitionRegistryBuilder, DefinitionValidationContext, OwnerId,
    },
    participant::ParticipantHandle,
};
use mush_org::SystemPrincipalId;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Trace = Arc<Mutex<Vec<DefinitionRef>>>;

fn failure() -> ModError {
    ModError::msg("renewal registration fixture mismatch")
}
fn descriptor(kind: &str, id: &str, version: u32) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|_| failure())?,
        DefinitionId::new(id).map_err(|_| failure())?,
        version,
        json!({"format":1,"registration_fixture":true}),
    )
    .map_err(|_| failure())
}
fn reference(owner: &str, descriptor: DefinitionDescriptor) -> Result<DefinitionRef, ModError> {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(owner).map_err(|_| failure())?, descriptor)
        .map_err(|_| failure())
}
fn policy() -> TestResult<InitialFilingPolicy> {
    Ok(InitialFilingPolicy::new(
        SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
        AccountId(11),
        Amount::from_minor(100),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )?)
}

#[derive(Clone)]
struct Decl {
    descriptor: DefinitionDescriptor,
    reference: DefinitionRef,
}
impl Decl {
    fn new(owner: &str, descriptor: DefinitionDescriptor) -> Result<Self, ModError> {
        let reference = reference(owner, descriptor.clone())?;
        Ok(Self {
            descriptor,
            reference,
        })
    }
}
#[derive(Clone)]
struct Specs {
    manifest: Decl,
    retention: Decl,
    economy: DefinitionRef,
    filing: Decl,
    original: [Decl; 3],
    derived: [Decl; 3],
    renewal: [Decl; 5],
}
impl Specs {
    fn new() -> TestResult<Self> {
        let manifest = Decl::new("tacsim", descriptor("registry_manifest", "combat", 1)?)?;
        let retention = Decl::new(
            "roll",
            descriptor("owner_participant", "commercial_roots", 1)?,
        )?;
        let economy = reference(
            "economy",
            DefinitionDescriptor::new(
                DefinitionKind::new("owner_participant")?,
                DefinitionId::new("initial_filing")?,
                1,
                policy()?.definition_payload(),
            )?,
        )?;
        let filing = Decl::new("roll", descriptor("action", "initial_filing", 1)?)?;
        let make = |id: &str, version: u32| Decl::new("market", descriptor("action", id, version)?);
        let original = [
            make("purchase_original_mind", 3)?,
            make("settle_original_mind", 3)?,
            make("refund_original_mind", 3)?,
        ];
        let derived = [
            make("purchase_derived_mind", 1)?,
            make("settle_derived_mind", 1)?,
            make("refund_derived_mind", 1)?,
        ];
        let make_renewal = |id: &str, transition: &str| -> Result<Decl, ModError> {
            Decl::new("market",DefinitionDescriptor::new(
                DefinitionKind::new("action").map_err(|_|failure())?,
                DefinitionId::new(id).map_err(|_|failure())?,1,
                json!({"format":1,"protocol":"personal_mind_renewal_v1","transition":transition}))
                .map_err(|_|failure())?.with_references(vec![
                    manifest.reference.clone(),economy.clone(),retention.reference.clone()]))
        };
        let renewal = [
            make_renewal(super::DRAFT_ACTION, "draft")?,
            make_renewal(super::FUND_ACTION, "fund")?,
            make_renewal(super::GRANT_ACTION, "grant")?,
            make_renewal(super::REFUND_ACTION, "refund")?,
            make_renewal(super::CLOSE_ACTION, "unfunded_close")?,
        ];
        Ok(Self {
            manifest,
            retention,
            economy,
            filing,
            original,
            derived,
            renewal,
        })
    }
    fn participants(&self) -> [DefinitionRef; 2] {
        [self.economy.clone(), self.retention.reference.clone()]
    }
    fn actions(&self) -> [DefinitionRef; 5] {
        self.renewal.clone().map(|d| d.reference)
    }
}
struct InertProvider {
    // The actual core-minted handles prove every declared action resolves to
    // the same real E4 adapter at final freeze; no owner request is executed.
    _economy: ParticipantHandle<InitialFilingParticipant>,
}
impl ActionProvider for InertProvider {
    fn validate_invocation(&self, _: &ActionInvocation) -> Result<(), ActionError> {
        Ok(())
    }
}
struct Definitions {
    owner: &'static str,
    declarations: Vec<Decl>,
    omit_provider: Option<DefinitionRef>,
    trace: Trace,
}
impl GameModule for Definitions {
    fn name(&self) -> &'static str {
        self.owner
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let mut kinds: BTreeMap<DefinitionKind, BTreeSet<DefinitionRef>> = BTreeMap::new();
        for d in &self.declarations {
            kinds
                .entry(d.reference.key().kind.clone())
                .or_default()
                .insert(d.reference.clone());
        }
        for (kind, expected) in kinds {
            reg.definition_validator(kind, move |context: &DefinitionValidationContext<'_>| {
                if expected.contains(context.definition().reference()) {
                    Ok(())
                } else {
                    Err("unexpected renewal fixture definition".into())
                }
            })?;
        }
        for d in &self.declarations {
            let actual = reg.definition(d.descriptor.clone())?;
            if actual != d.reference {
                return Err(failure());
            }
            if actual.key().kind.as_str() == "action" {
                let handle =
                    reg.request_owner_participant::<InitialFilingParticipant>(actual.clone())?;
                self.trace
                    .lock()
                    .map_err(|_| failure())?
                    .push(actual.clone());
                if self.omit_provider.as_ref() != Some(&actual) {
                    reg.action_provider(actual, InertProvider { _economy: handle })?;
                }
            }
        }
        Ok(())
    }
}
struct Economy {
    module: EconomyModule,
    duplicate: bool,
    specs: Specs,
}
impl GameModule for Economy {
    fn name(&self) -> &'static str {
        "economy"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.module.register(reg)?;
        if self.duplicate {
            // A second adapter for the same actual EconomyStore must refuse.
            let definition = reg.definition(
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant").map_err(|_| failure())?,
                    DefinitionId::new("duplicate_fixture").map_err(|_| failure())?,
                    1,
                    policy().map_err(|_| failure())?.definition_payload(),
                )
                .map_err(|_| failure())?,
            )?;
            reg.owner_participant(
                definition,
                BTreeSet::from([self.specs.filing.reference.clone()]),
                InitialFilingParticipant::new(policy().map_err(|_| failure())?),
            )?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum Fault {
    None,
    ManifestIdentity,
    ManifestDigest,
    EconomyParticipant,
    ParticipantOrder,
    RetentionDigest,
    ActionOrder,
    ActionDigest,
    ActionVersion,
    NoFiling,
    NoEntitlements,
    NoRenewalGrant,
    MissingManifest,
    MissingRetention,
    MissingProvider,
    DuplicateSlot,
}
fn modules(
    economy_first: bool,
    fault: Fault,
) -> TestResult<(Vec<Box<dyn GameModule>>, Specs, Trace)> {
    let specs = Specs::new()?;
    let mut manifest = specs.manifest.reference.clone();
    let mut participants = specs.participants();
    let mut actions = specs.actions();
    match fault {
        Fault::ManifestIdentity => {
            manifest = reference("elsewhere", descriptor("registry_manifest", "combat", 1)?)?
        }
        Fault::ManifestDigest => {
            manifest = reference(
                "tacsim",
                DefinitionDescriptor::new(
                    DefinitionKind::new("registry_manifest")?,
                    DefinitionId::new("combat")?,
                    1,
                    json!({"format":1,"changed":true}),
                )?,
            )?
        }
        Fault::EconomyParticipant => {
            *participants.first_mut().ok_or("economy participant")? = reference(
                "economy",
                descriptor("owner_participant", "initial_filing", 1)?,
            )?
        }
        Fault::ParticipantOrder => participants.swap(0, 1),
        Fault::RetentionDigest => {
            *participants.last_mut().ok_or("retention participant")? = reference(
                "roll",
                DefinitionDescriptor::new(
                    DefinitionKind::new("owner_participant")?,
                    DefinitionId::new("commercial_roots")?,
                    1,
                    json!({"format":1,"changed":true}),
                )?,
            )?
        }
        Fault::ActionOrder => actions.swap(0, 1),
        Fault::ActionDigest => {
            *actions.first_mut().ok_or("draft action")? =
                reference("market", descriptor("action", super::DRAFT_ACTION, 1)?)?
        }
        Fault::ActionVersion => {
            *actions.first_mut().ok_or("draft action")? =
                reference("market", descriptor("action", super::DRAFT_ACTION, 2)?)?
        }
        _ => {}
    }
    let mut economy = EconomyModule::new().with_title_issuers(vec![
        SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
    ]);
    if !matches!(fault, Fault::NoFiling) {
        economy = economy.with_initial_filing(policy()?, specs.filing.reference.clone());
    }
    if !matches!(fault, Fault::NoEntitlements) {
        economy = economy.with_mind_entitlement_purchases(
            specs.original.clone().map(|d| d.reference),
            specs.derived.clone().map(|d| d.reference),
        );
    }
    if !matches!(fault, Fault::NoRenewalGrant) {
        economy = economy.with_mind_renewals(manifest, participants, actions);
    }
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mut modules: Vec<Box<dyn GameModule>> = vec![Box::new(mush_org::OrgModule::new())];
    if !matches!(fault, Fault::MissingManifest) {
        modules.push(Box::new(Definitions {
            owner: "tacsim",
            declarations: vec![specs.manifest.clone()],
            omit_provider: None,
            trace: trace.clone(),
        }));
    }
    let mut roll = vec![specs.filing.clone()];
    if !matches!(fault, Fault::MissingRetention) {
        roll.push(specs.retention.clone());
    }
    modules.push(Box::new(Definitions {
        owner: "roll",
        declarations: roll,
        omit_provider: None,
        trace: trace.clone(),
    }));
    let declarations = specs
        .original
        .iter()
        .chain(&specs.derived)
        .chain(&specs.renewal)
        .cloned()
        .collect();
    modules.push(Box::new(Definitions {
        owner: "market",
        declarations,
        omit_provider: if matches!(fault, Fault::MissingProvider) {
            Some(
                specs
                    .renewal
                    .last()
                    .ok_or("close action")?
                    .reference
                    .clone(),
            )
        } else {
            None
        },
        trace: trace.clone(),
    }));
    // Positive and ordinary negative cases retain every real module hook.
    // Only duplicate registration needs the deliberately narrow wrapper.
    let economy: Box<dyn GameModule> = if matches!(fault, Fault::DuplicateSlot) {
        Box::new(Economy {
            module: economy,
            duplicate: true,
            specs: specs.clone(),
        })
    } else {
        Box::new(economy)
    };
    if economy_first {
        modules.insert(1, economy);
    } else {
        modules.push(economy);
    }
    Ok((modules, specs, trace))
}

fn attempt(
    economy_first: bool,
    fault: Fault,
) -> TestResult<(Result<Engine, ModError>, Specs, Trace)> {
    let (modules, specs, trace) = modules(economy_first, fault)?;
    Ok((
        Engine::with_modules(World::new_minimal(), Registry::new(), modules),
        specs,
        trace,
    ))
}

#[test]
fn renewal_bridge_resolves_all_actions_to_the_one_actual_economy_slot_in_either_order() -> TestResult
{
    let mut fingerprint = None;
    for first in [false, true] {
        let (result, specs, trace) = attempt(first, Fault::None)?;
        let engine = result?;
        let definitions = engine.registry.definitions().ok_or("definitions")?;
        definitions.get_exact(&specs.economy)?;
        assert_eq!(
            definitions
                .iter()
                .filter(|d| d.reference().key().owner.as_str() == "economy"
                    && d.reference().key().kind.as_str() == "owner_participant")
                .count(),
            1
        );
        let calls = trace.lock().map_err(|_| "trace poisoned")?;
        assert_eq!(calls.len(), 12);
        assert_eq!(calls.iter().cloned().collect::<BTreeSet<_>>().len(), 12);
        for action in specs.actions() {
            definitions.get_exact(&action)?;
            engine
                .registry
                .action_providers()
                .ok_or("providers")?
                .get_exact(&action)?;
            assert!(calls.contains(&action));
        }
        if let Some(old) = fingerprint {
            assert_eq!(definitions.fingerprint(), old);
        } else {
            fingerprint = Some(definitions.fingerprint());
        }
        assert!(engine.action_state().is_empty());
        assert!(!engine.action_state().has_prepared());
        assert!(!engine.action_state().reservation_lost());
        let store = engine
            .world
            .module_state::<EconomyStore>()
            .ok_or("actual Economy slot")?;
        assert_eq!(store.renewal_offers.operations().len(), 0);
        assert!(
            store.require_no_renewal_history().is_err(),
            "registration does not establish boot absence"
        );
    }
    Ok(())
}
#[test]
fn renewal_bridge_rejects_wrong_manifest_participant_order_or_digest() -> TestResult {
    for fault in [
        Fault::ManifestIdentity,
        Fault::ManifestDigest,
        Fault::EconomyParticipant,
        Fault::ParticipantOrder,
        Fault::RetentionDigest,
    ] {
        for first in [false, true] {
            let (result, _, _) = attempt(first, fault)?;
            assert!(result.is_err(), "crossed exact composition accepted");
        }
    }
    Ok(())
}
#[test]
fn renewal_bridge_rejects_swapped_or_rehashed_action_definitions() -> TestResult {
    for fault in [
        Fault::ActionOrder,
        Fault::ActionDigest,
        Fault::ActionVersion,
    ] {
        let (result, _, _) = attempt(false, fault)?;
        assert!(result.is_err(), "crossed exact action accepted");
    }
    Ok(())
}
#[test]
fn renewal_bridge_requires_existing_filing_and_entitlement_configuration() -> TestResult {
    for fault in [
        Fault::NoFiling,
        Fault::NoEntitlements,
        Fault::NoRenewalGrant,
    ] {
        for first in [false, true] {
            let (result, _, _) = attempt(first, fault)?;
            assert!(result.is_err(), "missing owner prerequisite accepted");
        }
    }
    Ok(())
}
#[test]
fn renewal_bridge_final_freeze_requires_actual_dependencies_and_every_provider() -> TestResult {
    for fault in [
        Fault::MissingManifest,
        Fault::MissingRetention,
        Fault::MissingProvider,
    ] {
        let (result, _, _) = attempt(true, fault)?;
        assert!(result.is_err(), "missing registered dependency accepted");
    }
    Ok(())
}
#[test]
fn renewal_bridge_cannot_register_a_second_adapter_for_the_existing_state_slot() -> TestResult {
    let (result, _, _) = attempt(true, Fault::DuplicateSlot)?;
    assert!(
        result.is_err(),
        "duplicate actual Economy participant accepted"
    );
    Ok(())
}

#[test]
fn renewal_bridge_configured_module_completes_actual_empty_boot() -> TestResult {
    let (modules, _, _) = modules(false, Fault::None)?;
    let world = mush_testkit::TestWorld::with_modules(modules);
    let state = world
        .world()
        .module_state::<EconomyStore>()
        .ok_or("actual Economy state")?;
    let audit = state.renewal_audit()?;
    audit.require_complete()?;
    assert!(audit.is_empty());
    state.require_no_renewal_history()?;
    assert_eq!(state.snapshot.contracts().count(), 0);
    assert_eq!(state.snapshot.holds().count(), 0);
    Ok(())
}
