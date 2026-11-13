//! The mock-owner skeleton of the composed owner tests (doc 46 cut CT.2).
//!
//! An owner test in `mush-economy` drives the real Economy owner the way a
//! caller module does: a mock module declares a few actions, registers a
//! provider for each, asks the owner to prepare inside the provider, and keeps
//! a typed **fixture receipt** of what the owner answered. The test then
//! prepares an action, stages it as an authoritative bundle, installs it, and
//! folds its replay writes into the documents a cold restart boots from.
//!
//! Five test files each wrote that skeleton out in full (the three
//! `owner_tests.rs` of `managed_coverage`, `managed_charter_transfer` and
//! `managed_paid_resale`, and the two `registry_filing` rigs). What differs
//! between them is the family: the provider's request, the receipt's subject,
//! the fixture objects and accounts. What does not differ is here:
//!
//! - [`MockOwner`] names one mock module: its owner id, its receipt family,
//!   its idempotency namespace, the words it refuses with;
//! - [`MockOwner::declare`] is the builder for its definitions, and
//!   [`register_declarations`] registers them with the validators that hold
//!   them to what was declared;
//! - [`FixtureReceipt`], [`MockOwner::receipts`] and
//!   [`MockOwner::prepared_receipt`] are the fixture receipt family: the strict
//!   typed row, and the check that a prepared action wrote exactly one of it;
//! - [`InertOwner`] and [`Unavailable`] are the prerequisite module whose
//!   actions exist and are never invoked;
//! - [`MockOwner::prepare`], [`MockOwner::stage`] and [`install_staged`] are
//!   the drive, with the same fixed request, offer and bundle hashes every rig
//!   used.
//!
//! This module names no commerce type: it is the kernel's vocabulary only, so
//! a crate's own unit tests can use it. A rig keeps its `Fixture` struct, its
//! provider and its receipt, and calls these.

use std::error::Error;
use std::fmt::Display;

use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, CanonicalHash, IdempotencyKey, IdempotencyScope, PerceivedRef,
    PreparedAuthoritativeAction, PreparedCausalRef, PrincipalId, RequestUid, ScopedIdempotencyKey,
};
use mush_core::action_state::{StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use mush_core::composed::PreparedComposedTxn;
use mush_core::mechanics::OwnerId;
use mush_core::{
    CanonicalId, CommitToken, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, DefinitionValidationContext, GameModule, ModError, ObjId, Registrar,
    RevisionSet,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::TestWorld;
use crate::fixtures::token;

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

/// A prepared composed action, the output its provider answered, and the
/// invocation that asked for it.
pub type PreparedAction = (
    PreparedComposedTxn,
    PreparedAuthoritativeAction,
    ActionInvocation,
);

/// One mock module's names. Every field is a literal of the rig that owns it.
#[derive(Clone, Copy, Debug)]
pub struct MockOwner {
    /// The module's name: the owner of its definitions and of its typed rows
    /// (`tacsim`, `market`, `roll`).
    pub owner: &'static str,
    /// The `fixture` label every definition's payload carries.
    pub label: &'static str,
    /// The receipt family's id (`coverage_fixture_receipt`); its schema ids
    /// are that id with `.key.v1`, `.value.v1` and the rest.
    pub receipts: &'static str,
    /// The idempotency namespace of the invocations (`coverage_fixture`).
    pub idempotency: &'static str,
    /// What the mock provider refuses with.
    pub refusal: &'static str,
}

/// The typed row a mock provider keeps of what the owner answered.
pub trait FixtureReceipt: Clone + Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The action the receipt says it was written by.
    fn action(&self) -> &DefinitionRef;
    /// The commit it says it was written under, which is also its key.
    fn commit(&self) -> CommitToken;
    /// The actor it says invoked the action.
    fn actor(&self) -> ObjId;
}

impl MockOwner {
    /// A definition of this fixture: `kind`/`id` at `version`, its payload the
    /// fixture label.
    ///
    /// # Errors
    ///
    /// A kind or id that is not canonical.
    pub fn descriptor(&self, kind: &str, id: &str, version: u32) -> Result<DefinitionDescriptor> {
        Ok(DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            version,
            json!({"fixture": self.label}),
        )?)
    }

    /// The builder for this module's definitions.
    ///
    /// # Errors
    ///
    /// An owner name that is not canonical.
    pub fn declare(&self) -> Result<Declarations> {
        Ok(Declarations {
            fixture: *self,
            owner: OwnerId::new(self.owner)?,
            builder: DefinitionRegistryBuilder::new(),
            declared: Vec::new(),
        })
    }

    /// The refusal the mock provider answers anything it will not prepare.
    ///
    /// # Panics
    ///
    /// Panics if [`MockOwner::refusal`] is not a valid error message, which is
    /// a broken fixture.
    #[must_use]
    pub fn error(&self) -> ActionError {
        ActionError::new(ActionErrorCode::ProviderRejected, false, self.refusal)
            .expect("static error")
    }

    /// Register the receipt family: keyed by the commit, strict on decode (a
    /// row must re-encode to exactly what was stored), with keys of at most
    /// `max_key_bytes` and values of at most `max_value_bytes`. `validate` is
    /// the receipt's own rule.
    ///
    /// # Errors
    ///
    /// The registrar's refusal.
    pub fn receipts<R, V>(
        &self,
        reg: &mut Registrar<'_>,
        max_key_bytes: usize,
        max_value_bytes: usize,
        validate: V,
    ) -> std::result::Result<StateFamilyHandle<CommitToken, R>, ModError>
    where
        R: FixtureReceipt,
        V: Fn(&R) -> std::result::Result<(), String> + Send + Sync + 'static,
    {
        let bad = |e: &dyn Display| ModError::msg(e.to_string());
        let id = |part: &str| {
            CanonicalId::new(format!("{}.{part}.v1", self.receipts)).map_err(|e| bad(&e))
        };
        reg.state_family(
            StateFamilyDescriptor::new(
                StateFamilyId::new(self.receipts).map_err(|e| bad(&e))?,
                id("key")?,
                id("value")?,
                1,
                id("decode")?,
                id("validate")?,
                id("canonical")?,
            )
            .map_err(|e| bad(&e))?
            .with_limits(max_key_bytes, max_value_bytes)
            .map_err(|e| bad(&e))?,
            |key: &CommitToken| Ok(key.to_hex()),
            |value: &Value| {
                let row: R = serde_json::from_value(value.clone())
                    .map_err(|_| "fixture receipt encoding".to_owned())?;
                if serde_json::to_value(&row).map_err(|_| "fixture receipt encoding".to_owned())?
                    != *value
                {
                    return Err("fixture receipt exact shape".into());
                }
                Ok(row)
            },
            validate,
            |row: &R| serde_json::to_value(row).map_err(|_| "fixture receipt encoding".into()),
        )
    }

    /// The one fixture receipt among a prepared action's `writes`: it must be
    /// valid, be stored under `commit` in this module's family with exactly
    /// the typed envelope, and name `action`, `commit` and `actor`.
    ///
    /// # Errors
    ///
    /// No receipt, more than one, or one that differs in any of those.
    pub fn prepared_receipt<R, V>(
        &self,
        writes: &[ReplayWrite],
        action: &DefinitionRef,
        commit: CommitToken,
        actor: ObjId,
        validate: V,
    ) -> Result<R>
    where
        R: FixtureReceipt,
        V: Fn(&R) -> std::result::Result<(), String>,
    {
        let mut found = None;
        for write in writes {
            let ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } = write
            else {
                continue;
            };
            if namespace.as_str() != self.owner {
                continue;
            }
            let receipt: R =
                serde_json::from_value(value.get("value").ok_or("typed fixture value")?.clone())?;
            validate(&receipt)?;
            let expected = json!({
                "family": {"owner": self.owner, "id": self.receipts},
                "key": receipt.commit().to_hex(),
                "schema": {"id": format!("{}.value.v1", self.receipts), "version": 1},
                "value": receipt,
            });
            if value != &expected
                || key.as_str() != format!("action-state/v1/{}/{}", self.receipts, commit.to_hex())
                || receipt.action() != action
                || receipt.commit() != commit
                || receipt.actor() != actor
                || found.is_some()
            {
                return Err("unexpected/crossed typed fixture write".into());
            }
            found = Some(receipt);
        }
        found.ok_or_else(|| "actual composed fixture receipt missing".into())
    }

    /// Prepare `action` for `actor` with `parameters`, as request and commit
    /// `n` (sixteen bytes of `n` each), through the registered provider.
    ///
    /// # Errors
    ///
    /// The invocation's shape, or whatever the provider and the owners it
    /// asked refused.
    pub fn prepare(
        &self,
        world: &mut TestWorld,
        action: &DefinitionRef,
        actor: ObjId,
        n: u8,
        parameters: Value,
    ) -> Result<PreparedAction> {
        let commit = token(n);
        let request = RequestUid::from_bytes([n; 16]);
        let principal = PrincipalId::Object { id: actor };
        let inv = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new(self.owner)?,
                    namespace: CanonicalId::new(self.idempotency)?,
                },
                1,
                IdempotencyKey::Request(request),
            ),
        })?;
        let (p, o) = world
            .engine_mut()
            .prepare_registered_composed_authoritative_action(action, &inv, commit)?;
        Ok((p, o, inv))
    }

    /// Stage a prepared action as the authoritative bundle of commit `n`. The
    /// fixture receipt is read out of its replay writes first
    /// ([`MockOwner::prepared_receipt`]) and returned with them; the caller
    /// verifies it against the owner's state once the action is installed.
    ///
    /// # Errors
    ///
    /// A missing or crossed receipt, a bundle the kernel refuses, or a
    /// refused staging.
    pub fn stage<R, V>(
        &self,
        world: &mut TestWorld,
        action: DefinitionRef,
        n: u8,
        prepared: PreparedAction,
        validate: V,
    ) -> Result<(Vec<ReplayWrite>, R)>
    where
        R: FixtureReceipt,
        V: Fn(&R) -> std::result::Result<(), String>,
    {
        let (p, o, inv) = prepared;
        let writes = p.authoritative_replay_writes();
        let receipt = self.prepared_receipt(&writes, &action, token(n), inv.actor(), validate)?;
        let (outcome, resolution, mut definitions, mut witnesses, mut outbox, events) =
            o.into_parts();
        definitions.push(action.clone());
        definitions.push(outcome.schema().clone());
        definitions.sort();
        definitions.dedup();
        if witnesses.is_empty() {
            witnesses.push(inv.principal().clone());
        }
        witnesses.sort();
        witnesses.dedup();
        outbox.sort_by_key(mush_core::DurableOutboxIntent::ordinal);
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(n),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([3; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([4; 32]),
            action,
            provider: p.owner().clone(),
            cause: PreparedCausalRef::new(inv.request(), None, 0)?,
            definitions,
            writes: writes.clone(),
            outcome,
            resolution: resolution.map(|r| r.bind(token(n))),
            semantic_events: events,
            eligible_witnesses: witnesses,
            durable_outbox: outbox,
        })?;
        let guards = world
            .registry()
            .server_guard_set_builder(RevisionSet::default())?
            .finish()?;
        world
            .engine_mut()
            .stage_authoritative_action(p, guards, bundle)?;
        Ok((writes, receipt))
    }
}

/// Take the staged action and install it: the final owner and guard recheck,
/// then the live mutation.
///
/// # Errors
///
/// Nothing staged, a recheck that no longer holds, or a refused install.
pub fn install_staged(world: &mut TestWorld) -> Result {
    let held = world
        .engine_mut()
        .take_staged_action()?
        .ok_or("actual staged action")?;
    let (_, installation) = held.into_parts();
    world.engine_mut().install_composed_action(installation)?;
    Ok(())
}

/// The builder for a mock module's definitions: each one is registered as it
/// is added, so its reference is in hand for the rig, and kept for
/// [`register_declarations`].
pub struct Declarations {
    fixture: MockOwner,
    owner: OwnerId,
    builder: DefinitionRegistryBuilder,
    declared: Vec<DefinitionDescriptor>,
}

impl Declarations {
    /// Declare `kind`/`id` at `version` and answer its reference.
    ///
    /// # Errors
    ///
    /// A name that is not canonical, or a definition declared twice.
    pub fn add(&mut self, kind: &str, id: &str, version: u32) -> Result<DefinitionRef> {
        let descriptor = self.fixture.descriptor(kind, id, version)?;
        let reference = self
            .builder
            .register(self.owner.clone(), descriptor.clone())?;
        self.declared.push(descriptor);
        Ok(reference)
    }

    /// Every definition declared, in order.
    #[must_use]
    pub fn finish(self) -> Vec<DefinitionDescriptor> {
        self.declared
    }
}

/// Register `declarations` from a module's `register`: for each of `kinds` a
/// validator that accepts a definition only if its id, payload and references
/// are those of a declaration of that kind, then the definitions themselves.
///
/// # Errors
///
/// The registrar's refusal.
pub fn register_declarations(
    reg: &mut Registrar<'_>,
    declarations: &[DefinitionDescriptor],
    kinds: &[&str],
) -> std::result::Result<(), ModError> {
    for kind in kinds {
        let expected: Vec<_> = declarations
            .iter()
            .filter(|d| d.kind().as_str() == *kind)
            .cloned()
            .collect();
        reg.definition_validator(
            DefinitionKind::new(*kind).map_err(|e| ModError::msg(e.to_string()))?,
            move |ctx: &DefinitionValidationContext<'_>| {
                let e = ctx.definition();
                if expected.iter().any(|d| {
                    d.id() == &e.reference().key().id
                        && d.payload() == e.payload()
                        && d.references() == e.references()
                }) {
                    Ok(())
                } else {
                    Err("fixture definition drift".into())
                }
            },
        )?;
    }
    for d in declarations {
        reg.definition(d.clone())?;
    }
    Ok(())
}

/// A provider whose action exists and refuses every invocation.
pub struct Unavailable(pub ActionError);

impl ActionProvider for Unavailable {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Err(self.0.clone())
    }
}

/// A module that declares actions a configuration needs to exist and never
/// lets one be invoked: the prerequisite owner a rig composes and does not
/// exercise (the `roll` module of the coverage and paid-resale rigs, whose
/// initial-filing action Economy's configuration names).
pub struct InertOwner {
    name: &'static str,
    actions: Vec<DefinitionDescriptor>,
    refusal: ActionError,
}

impl InertOwner {
    /// A module named `name` declaring `actions`, each refusing with
    /// `refusal`.
    #[must_use]
    pub fn new(
        name: &'static str,
        actions: Vec<DefinitionDescriptor>,
        refusal: ActionError,
    ) -> Self {
        Self {
            name,
            actions,
            refusal,
        }
    }
}

impl GameModule for InertOwner {
    fn name(&self) -> &'static str {
        self.name
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let expected = self.actions.clone();
        reg.definition_validator(
            DefinitionKind::new("action").map_err(|e| ModError::msg(e.to_string()))?,
            move |ctx: &DefinitionValidationContext<'_>| {
                let e = ctx.definition();
                if expected.iter().any(|d| {
                    d.id() == &e.reference().key().id
                        && d.payload() == e.payload()
                        && d.references() == e.references()
                }) {
                    Ok(())
                } else {
                    Err("fixture definition drift".into())
                }
            },
        )?;
        for d in &self.actions {
            let action = reg.definition(d.clone())?;
            reg.action_provider(action, Unavailable(self.refusal.clone()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::action::AuthoritativeActionOutput;
    use mush_core::{ActionTxn, GOD, PrepareCtx, RulesValue};
    use serde::Deserialize;

    use std::sync::{Arc, Mutex};

    use crate::cold::{Boot, fold_replay_writes_into};
    use crate::{check, check_eq};

    const TOY: MockOwner = MockOwner {
        owner: "toy",
        label: "toy-owner",
        receipts: "toy_fixture_receipt",
        idempotency: "toy_fixture",
        refusal: "toy owner fixture refused",
    };

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        format: u8,
        action: DefinitionRef,
        commit: CommitToken,
        actor: ObjId,
        note: String,
    }
    impl FixtureReceipt for Receipt {
        fn action(&self) -> &DefinitionRef {
            &self.action
        }
        fn commit(&self) -> CommitToken {
            self.commit
        }
        fn actor(&self) -> ObjId {
            self.actor
        }
    }
    fn valid(row: &Receipt) -> std::result::Result<(), String> {
        if row.format == 1 && row.note != "invalid" {
            Ok(())
        } else {
            Err("toy receipt".into())
        }
    }

    type Handle = StateFamilyHandle<CommitToken, Receipt>;

    #[derive(Clone)]
    struct Toy {
        declarations: Vec<DefinitionDescriptor>,
        write: DefinitionRef,
        cross: DefinitionRef,
        outcome: DefinitionRef,
        handle: Arc<Mutex<Option<Handle>>>,
    }
    fn toy() -> Result<Toy> {
        let mut d = TOY.declare()?;
        let write = d.add("action", "toy_write", 1)?;
        let cross = d.add("action", "toy_cross", 1)?;
        let outcome = d.add("outcome", "toy_outcome", 1)?;
        Ok(Toy {
            declarations: d.finish(),
            write,
            cross,
            outcome,
            handle: Arc::default(),
        })
    }

    /// Writes one receipt naming its own action, or, as `cross`, naming the
    /// other one: the crossed write `prepared_receipt` must refuse.
    struct Provider {
        receipts: StateFamilyHandle<CommitToken, Receipt>,
        names: DefinitionRef,
        outcome: DefinitionRef,
    }
    impl ActionProvider for Provider {
        fn validate_invocation(
            &self,
            _: &ActionInvocation,
        ) -> std::result::Result<(), ActionError> {
            Ok(())
        }
        fn prepare_authoritative_with_context(
            &self,
            inv: &ActionInvocation,
            txn: &mut ActionTxn<'_>,
            ctx: &mut PrepareCtx<'_>,
        ) -> std::result::Result<AuthoritativeActionOutput, ActionError> {
            let fail = |what: &str| {
                ActionError::new(ActionErrorCode::ProviderRejected, false, what)
                    .unwrap_or_else(|_| TOY.error())
            };
            let note = inv
                .parameters()
                .as_str()
                .ok_or_else(|| fail("toy parameters"))?
                .to_owned();
            if note == "refuse" {
                return Err(TOY.error());
            }
            txn.insert(
                &self.receipts,
                &ctx.commit_token(),
                Receipt {
                    format: 1,
                    action: self.names.clone(),
                    commit: ctx.commit_token(),
                    actor: inv.actor(),
                    note: note.clone(),
                },
            )
            .map_err(|e| fail(&format!("toy insert: {e}")))?;
            Ok(AuthoritativeActionOutput::new(
                RulesValue::new(self.outcome.clone(), json!({"note": note}))
                    .map_err(|e| fail(&format!("toy outcome: {e}")))?,
            )
            .with_definitions(vec![self.outcome.clone()]))
        }
    }
    impl GameModule for Toy {
        fn name(&self) -> &'static str {
            "toy"
        }
        fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
            register_declarations(reg, &self.declarations, &["action", "outcome"])?;
            let receipts = TOY.receipts::<Receipt, _>(reg, 128, 4096, valid)?;
            *self
                .handle
                .lock()
                .map_err(|_| ModError::msg("handle lock"))? = Some(receipts.clone());
            reg.action_provider(
                self.write.clone(),
                Provider {
                    receipts: receipts.clone(),
                    names: self.write.clone(),
                    outcome: self.outcome.clone(),
                },
            )?;
            reg.action_provider(
                self.cross.clone(),
                Provider {
                    receipts,
                    names: self.write.clone(),
                    outcome: self.outcome.clone(),
                },
            )
        }
    }

    /// The rig's module. A clone shares the handle slot, so the last engine
    /// composed is the one `stored` reads; every test reads the engine it
    /// composed last.
    fn rig() -> Result<(TestWorld, Toy)> {
        let toy = toy()?;
        let mut world = TestWorld::with_modules(vec![Box::new(toy.clone())]);
        // Staging refuses a world with undrained legacy writes (God's connect
        // is one), so a rig drains its setup first.
        let _ = world.take_module_ops();
        let _ = world.world_mut().take_dirty();
        Ok((world, toy))
    }

    fn stored(world: &TestWorld, toy: &Toy, n: u8) -> Result<Option<Receipt>> {
        let handle = toy
            .handle
            .lock()
            .map_err(|_| "handle lock")?
            .clone()
            .ok_or("the family was registered")?;
        Ok(world
            .engine()
            .action_state()
            .get(&handle, &token(n))?
            .cloned())
    }

    #[test]
    fn a_prepared_action_is_staged_installed_and_survives_a_cold_restart() -> Result {
        let (mut world, toy) = rig()?;
        let prepared = TOY.prepare(&mut world, &toy.write, GOD, 2, json!("kept"))?;
        let (writes, receipt) =
            TOY.stage::<Receipt, _>(&mut world, toy.write.clone(), 2, prepared, valid)?;
        check_eq!(receipt.note, "kept");
        check_eq!(receipt.commit, token(2));
        check_eq!(receipt.actor, GOD);
        check_eq!(stored(&world, &toy, 2)?, None, "staged is not installed");
        install_staged(&mut world)?;
        check_eq!(stored(&world, &toy, 2)?, Some(receipt.clone()));

        let mut boot = Boot::new();
        fold_replay_writes_into(&mut boot, &writes)?;
        let cold = world.cold_restart(vec![Box::new(toy.clone())], boot)?;
        check_eq!(stored(&cold, &toy, 2)?, Some(receipt));
        // A restart from documents that never took the write does not hold it.
        let cold = world.cold_restart(vec![Box::new(toy.clone())], Boot::new())?;
        check_eq!(stored(&cold, &toy, 2)?, None);
        Ok(())
    }

    #[test]
    fn a_refusing_provider_refuses_the_prepare() -> Result {
        let (mut world, toy) = rig()?;
        check!(
            TOY.prepare(&mut world, &toy.write, GOD, 2, json!("refuse"))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn a_receipt_that_names_another_action_is_refused_at_staging() -> Result {
        let (mut world, toy) = rig()?;
        let prepared = TOY.prepare(&mut world, &toy.cross, GOD, 2, json!("crossed"))?;
        let Err(error) = TOY.stage::<Receipt, _>(&mut world, toy.cross.clone(), 2, prepared, valid)
        else {
            return Err("a receipt naming another action must not stage".into());
        };
        check_eq!(error.to_string(), "unexpected/crossed typed fixture write");
        Ok(())
    }

    #[test]
    fn a_receipt_its_own_rule_refuses_is_refused_at_staging() -> Result {
        let (mut world, toy) = rig()?;
        let prepared = TOY.prepare(&mut world, &toy.write, GOD, 2, json!("kept"))?;
        let Err(error) =
            TOY.stage::<Receipt, _>(&mut world, toy.write.clone(), 2, prepared, |_| {
                Err("the caller's rule".into())
            })
        else {
            return Err("the caller's rule must hold the staging".into());
        };
        check_eq!(error.to_string(), "the caller's rule");
        Ok(())
    }

    #[test]
    fn nothing_staged_is_nothing_to_install() -> Result {
        let (mut world, _) = rig()?;
        let Err(error) = install_staged(&mut world) else {
            return Err("nothing was staged".into());
        };
        check_eq!(error.to_string(), "actual staged action");
        Ok(())
    }

    #[test]
    fn an_inert_owners_action_exists_and_is_never_available() -> Result {
        let inert = MockOwner {
            owner: "inert",
            ..TOY
        };
        let descriptor = inert.descriptor("action", "initial_filing", 1)?;
        let action = DefinitionRegistryBuilder::new()
            .register(OwnerId::new("inert")?, descriptor.clone())?;
        let mut world = TestWorld::with_modules(vec![Box::new(InertOwner::new(
            "inert",
            vec![descriptor],
            inert.error(),
        ))]);
        check!(
            inert
                .prepare(&mut world, &action, GOD, 2, json!(null))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn a_definition_that_drifted_from_its_declaration_does_not_compose() -> Result {
        struct Drifted(Toy);
        impl GameModule for Drifted {
            fn name(&self) -> &'static str {
                "toy"
            }
            fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
                register_declarations(reg, &self.0.declarations, &["action", "outcome"])?;
                let stray = MockOwner {
                    label: "another-label",
                    ..TOY
                }
                .descriptor("action", "toy_stray", 1)
                .map_err(|e| ModError::msg(e.to_string()))?;
                reg.definition(stray).map(|_| ())
            }
        }
        let (world, registry) = crate::boot_world();
        let composed =
            mush_core::Engine::with_modules(world, registry, vec![Box::new(Drifted(toy()?))]);
        check!(
            composed.is_err(),
            "an undeclared definition must not compose"
        );
        Ok(())
    }
}
