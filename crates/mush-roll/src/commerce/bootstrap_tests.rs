#![cfg(test)]

//! Real registered setup/installation/restart boundaries. The only fixture
//! adapters are an empty source owner and a registered game manifest; no test
//! sets the economy receipt-proof bit or opens commercial readiness directly.

use std::collections::HashMap;

use mush_core::action::{
    ActionInvocation, ActionInvocationDraft, ActionOfferId, CanonicalHash, CommitToken,
    IdempotencyKey, IdempotencyScope, PerceivedRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
};
use mush_core::authoritative::ReplayWrite;
use mush_core::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{
    ActionStateBootRecord, Engine, GOD, GameModule, ModError, ModuleBootData, ModuleCtx, Outbox,
    Registrar, Registry, World,
};
use mush_economy::{AccountId, Amount, CatalogRef, EconomyModule, EconomyStore};
use serde_json::json;

use super::filing::InitialFilingTerms;
use super::module::{CommerceConfig, CommerceMode};
use super::rating::RatingCompatibility;
use crate::{
    Envelope, PreparedSource, RevisionAdapter, RollError, RollModule, RollStore, SourceAdapter,
};

pub(super) fn descriptor(kind: &str, id: &str, value: serde_json::Value) -> DefinitionDescriptor {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).unwrap(),
        DefinitionId::new(id).unwrap(),
        1,
        value,
    )
    .unwrap()
}
pub(super) fn reference(owner: &str, descriptor: DefinitionDescriptor) -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(OwnerId::new(owner).unwrap(), descriptor)
        .unwrap()
}
pub(super) fn manifest_descriptor() -> DefinitionDescriptor {
    // This registers an actual immutable fixture declaration, not a rating or
    // a counterfeit successful producer receipt. Every rating family is empty.
    descriptor(
        "registry_manifest",
        "combat",
        json!({"fixture":"empty-recovery"}),
    )
}
pub(super) fn setup_action() -> DefinitionRef {
    reference(
        "roll",
        descriptor(
            "action",
            "initialize_commerce",
            json!({"format":1,"explicit_setup":true}),
        ),
    )
}
pub(super) struct Manifest;
impl GameModule for Manifest {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let expected = vec![manifest_descriptor()];
        reg.definition_validator(
            DefinitionKind::new("registry_manifest").map_err(|e| ModError::msg(e.to_string()))?,
            move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
                super::module::validate_definitions("tacsim", &expected, ctx)
            },
        )?;
        reg.definition(manifest_descriptor())?;
        Ok(())
    }
}
struct EmptySources;
impl RevisionAdapter for EmptySources {
    fn normalize_source(&self, _: &str) -> Result<String, RollError> {
        Err(RollError::Missing)
    }
    fn validate(&self, _: &str, _: &Envelope) -> Result<(), RollError> {
        Err(RollError::Missing)
    }
}
impl SourceAdapter for EmptySources {
    type Token = ();
    fn prepare_source(&self, _: &str, _: &str) -> Result<PreparedSource<()>, RollError> {
        Err(RollError::Missing)
    }
    fn read_source(&self, _: mush_core::DefinitionDigest) -> Result<&str, RollError> {
        Err(RollError::Missing)
    }
}
struct SourceHydratedRoll(RollModule);
impl GameModule for SourceHydratedRoll {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.0.register(reg)
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.0.on_world_loaded(ctx, data)
    }
    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        // Run the real source audit. Any unexpected revision fails because the
        // fixture has no source; successful empty audit is its genuine result.
        ctx.state_mut::<RollStore>()
            .ok_or_else(|| ModError::msg("missing Roll"))?
            .audit_sources(&EmptySources, &EmptySources)
            .map_err(|e| ModError::msg(e.to_string()))
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.0.finalize_owner_composition(ctx)
    }
}
pub(super) fn engine(mode: CommerceMode) -> Engine {
    let manifest = reference("tacsim", manifest_descriptor());
    let config = CommerceConfig::new(
        mode,
        InitialFilingTerms::new(90 * 24 * 60 * 60, 3 * 24 * 60 * 60).unwrap(),
        manifest.clone(),
        RatingCompatibility::combat_v1(manifest.digest()),
    )
    .unwrap();
    let action = config.initial_filing_action().unwrap();
    let issuer = mush_org::SystemPrincipalId::new("roll.registry").unwrap();
    let policy = mush_economy::registry_filing::InitialFilingPolicy::new(
        issuer.clone(),
        AccountId(1),
        Amount::from_minor(100),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )
    .unwrap();
    Engine::with_modules(
        World::new_minimal(),
        Registry::new(),
        vec![
            Box::new(Manifest),
            Box::new(SourceHydratedRoll(
                RollModule::with_writers(["tacsim"]).with_commerce(config),
            )),
            Box::new(
                EconomyModule::new()
                    .with_title_issuers(vec![issuer])
                    .with_initial_filing(policy, action),
            ),
        ],
    )
    .unwrap()
}
pub(super) fn boot(engine: &mut Engine) -> Result<(), ModError> {
    engine.modules_loaded(
        HashMap::new(),
        &mut Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )
}
pub(super) fn invocation() -> ActionInvocation {
    let request = RequestUid::from_bytes([1; 16]);
    let principal = PrincipalId::Object { id: GOD };
    ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor: GOD,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([3; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("roll").unwrap(),
                namespace: CanonicalId::new("setup").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap()
}
fn hydrate(engine: &mut Engine, writes: &[ReplayWrite]) {
    let rows = writes
        .iter()
        .map(|write| {
            let ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } = write
            else {
                panic!("setup only writes typed markers")
            };
            ActionStateBootRecord::new(namespace.as_str(), key.as_str(), value.clone())
        })
        .collect::<Vec<_>>();
    engine.hydrate_action_state(rows).unwrap();
}
fn assert_closed(engine: &Engine) {
    let roll = engine.world.module_state::<RollStore>().unwrap();
    assert!(roll.commerce_ready().is_err());
    assert!(roll.prepare_prune(1).is_err());
}

#[test]
fn setup_uses_actual_empty_economy_replay_and_does_not_open_until_active_restart() {
    let mut active = engine(CommerceMode::Active);
    assert!(
        boot(&mut active).is_err(),
        "missing deployment is not initial setup"
    );
    assert_closed(&active);

    let mut initialize = engine(CommerceMode::Initialize);
    assert!(
        initialize
            .world
            .module_state::<EconomyStore>()
            .unwrap()
            .filing_receipt_audit()
            .require_complete()
            .is_err()
    );
    boot(&mut initialize).unwrap();
    assert!(
        initialize
            .world
            .module_state::<EconomyStore>()
            .unwrap()
            .filing_receipt_audit()
            .is_empty()
            .unwrap()
    );
    assert_closed(&initialize);
    assert!(
        initialize
            .registry
            .persist_delegation("tacsim", "roll")
            .is_some(),
        "opt-in preserves existing M6 source/revision writer"
    );

    let commit = CommitToken::from_bytes([9; 16]);
    let (prepared, _) = initialize
        .prepare_registered_composed_authoritative_action(&setup_action(), &invocation(), commit)
        .unwrap();
    assert!(
        initialize.action_state().is_empty(),
        "prepare installs neither marker"
    );
    let (writes, installation) = prepared.into_parts();
    assert_eq!(
        writes.len(),
        2,
        "one Roll and one shell deployment share a commit"
    );
    for write in &writes {
        let ReplayWrite::ModuleJsonSet {
            namespace, value, ..
        } = write
        else {
            panic!("typed marker")
        };
        assert_eq!(namespace.as_str(), "roll");
        assert_eq!(
            value.get("value").unwrap().get("initialized_by"),
            Some(&json!(commit))
        );
    }
    initialize.install_composed_action(installation).unwrap();
    assert_closed(&initialize);
    assert!(
        initialize
            .prepare_registered_composed_authoritative_action(
                &setup_action(),
                &invocation(),
                CommitToken::from_bytes([10; 16])
            )
            .is_err(),
        "setup cannot overwrite an initialized deployment"
    );

    let mut restarted = engine(CommerceMode::Active);
    hydrate(&mut restarted, &writes);
    boot(&mut restarted).unwrap();
    assert!(
        restarted
            .world
            .module_state::<RollStore>()
            .unwrap()
            .commerce_ready()
            .is_ok()
    );
    assert!(
        restarted
            .world
            .module_state::<RollStore>()
            .unwrap()
            .prepare_prune(1)
            .is_ok()
    );
    let mut setup_again = engine(CommerceMode::Initialize);
    hydrate(&mut setup_again, &writes);
    assert!(
        boot(&mut setup_again).is_err(),
        "Initialize refuses retained prior deployment"
    );
    assert_closed(&setup_again);
}

#[test]
fn uninstalled_setup_has_no_half_state_and_exact_persisted_bundle_recovers() {
    let mut original = engine(CommerceMode::Initialize);
    boot(&mut original).unwrap();
    let (prepared, _) = original
        .prepare_registered_composed_authoritative_action(
            &setup_action(),
            &invocation(),
            CommitToken::from_bytes([8; 16]),
        )
        .unwrap();
    let (writes, installation) = prepared.into_parts();
    drop(installation);
    assert!(original.action_state().is_empty());
    assert!(
        original.action_state().reservation_lost(),
        "uninstalled custody is fail-stop"
    );
    assert_closed(&original);

    // Model durable write replay after no RAM installation. The server's actual
    // lost-ACK protocol is a separate integration gate, not claimed here.
    let mut recovered = engine(CommerceMode::Active);
    hydrate(&mut recovered, &writes);
    boot(&mut recovered).unwrap();
    assert!(
        recovered
            .world
            .module_state::<RollStore>()
            .unwrap()
            .commerce_ready()
            .is_ok()
    );
    for omitted in 0..writes.len() {
        let partial: Vec<_> = writes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != omitted)
            .map(|(_, w)| w.clone())
            .collect();
        let mut broken = engine(CommerceMode::Active);
        hydrate(&mut broken, &partial);
        assert!(
            boot(&mut broken).is_err(),
            "either missing marker refuses recovery"
        );
        assert_closed(&broken);
    }
    let mut refused_without_persistence = engine(CommerceMode::Initialize);
    boot(&mut refused_without_persistence).unwrap();
    assert!(refused_without_persistence.action_state().is_empty());
    assert_closed(&refused_without_persistence);
}
